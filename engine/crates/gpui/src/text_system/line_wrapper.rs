use crate::{FontId, Pixels, SharedString, TextRun, TextSystem, px};
use collections::HashMap;
use std::{borrow::Cow, iter, sync::Arc};
use unicode_general_category::{GeneralCategory, get_general_category};

/// Determines whether to truncate text from the start or end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TruncateFrom {
    /// Truncate text from the start.
    Start,
    /// Truncate text from the end.
    End,
    /// Truncate text from the middle, preserving the start and end.
    Middle,
}

/// The GPUI line wrapper, used to wrap lines of text to a given width.
pub struct LineWrapper {
    text_system: Arc<TextSystem>,
    pub(crate) font_id: FontId,
    pub(crate) font_size: Pixels,
    cached_ascii_char_widths: [Option<Pixels>; 128],
    cached_other_char_widths: HashMap<char, Pixels>,
}

impl LineWrapper {
    /// The maximum indent that can be applied to a line.
    pub const MAX_INDENT: u32 = 256;

    pub(crate) fn new(font_id: FontId, font_size: Pixels, text_system: Arc<TextSystem>) -> Self {
        Self {
            text_system,
            font_id,
            font_size,
            cached_ascii_char_widths: [None; 128],
            cached_other_char_widths: HashMap::default(),
        }
    }

    /// Wrap a line of text to the given width with this wrapper's font and font size.
    pub fn wrap_line<'a>(
        &'a mut self,
        fragments: &'a [LineFragment],
        wrap_width: Pixels,
    ) -> impl Iterator<Item = Boundary> + 'a {
        let mut width = px(0.);
        let mut first_non_whitespace_ix = None;
        let mut indent = None;
        let mut last_candidate_ix = 0;
        let mut last_candidate_width = px(0.);
        let mut last_wrap_ix = 0;
        let mut prev_c = '\0';
        let mut index = 0;
        let mut candidates = fragments
            .iter()
            .flat_map(move |fragment| fragment.wrap_boundary_candidates())
            .peekable();
        iter::from_fn(move || {
            for candidate in candidates.by_ref() {
                let ix = index;
                index += candidate.len_utf8();
                let mut new_prev_c = prev_c;
                let item_width = match candidate {
                    WrapBoundaryCandidate::Char { character: c } => {
                        if c == '\n' {
                            continue;
                        }

                        if Self::is_word_char(c) {
                            if prev_c == ' ' && c != ' ' && first_non_whitespace_ix.is_some() {
                                last_candidate_ix = ix;
                                last_candidate_width = width;
                            }
                        } else {
                            // CJK may not be space separated, e.g.: `Hello world你好世界`
                            if c != ' ' && first_non_whitespace_ix.is_some() {
                                last_candidate_ix = ix;
                                last_candidate_width = width;
                            }
                        }

                        if c != ' ' && first_non_whitespace_ix.is_none() {
                            first_non_whitespace_ix = Some(ix);
                        }

                        new_prev_c = c;

                        self.width_for_char(c)
                    }
                    WrapBoundaryCandidate::Element {
                        width: element_width,
                        ..
                    } => {
                        if prev_c == ' ' && first_non_whitespace_ix.is_some() {
                            last_candidate_ix = ix;
                            last_candidate_width = width;
                        }

                        if first_non_whitespace_ix.is_none() {
                            first_non_whitespace_ix = Some(ix);
                        }

                        element_width
                    }
                };

                width += item_width;
                if width > wrap_width && ix > last_wrap_ix {
                    if let (None, Some(first_non_whitespace_ix)) = (indent, first_non_whitespace_ix)
                    {
                        indent = Some(
                            Self::MAX_INDENT.min((first_non_whitespace_ix - last_wrap_ix) as u32),
                        );
                    }

                    if last_candidate_ix > 0 {
                        last_wrap_ix = last_candidate_ix;
                        width -= last_candidate_width;
                        last_candidate_ix = 0;
                    } else {
                        last_wrap_ix = ix;
                        width = item_width;
                    }

                    if let Some(indent) = indent {
                        width += self.width_for_char(' ') * indent as f32;
                    }

                    return Some(Boundary::new(last_wrap_ix, indent.unwrap_or(0)));
                }

                prev_c = new_prev_c;
            }

            None
        })
    }

    /// Determines if a line should be truncated based on its width.
    ///
    /// Returns the truncation index in `line`.
    pub fn should_truncate_line(
        &mut self,
        line: &str,
        truncate_width: Pixels,
        truncation_affix: &str,
        truncate_from: TruncateFrom,
    ) -> Option<usize> {
        let mut width = px(0.);
        let suffix_width = truncation_affix
            .chars()
            .map(|c| self.width_for_char(c))
            .fold(px(0.0), |a, x| a + x);
        let mut truncate_ix = 0;

        match truncate_from {
            TruncateFrom::Start => {
                for (ix, c) in line.char_indices().rev() {
                    if width + suffix_width < truncate_width {
                        truncate_ix = ix;
                    }

                    let char_width = self.width_for_char(c);
                    width += char_width;

                    if width.floor() > truncate_width {
                        return Some(truncate_ix);
                    }
                }
            }
            TruncateFrom::End => {
                for (ix, c) in line.char_indices() {
                    if width + suffix_width < truncate_width {
                        truncate_ix = ix;
                    }

                    let char_width = self.width_for_char(c);
                    width += char_width;

                    if width.floor() > truncate_width {
                        return Some(truncate_ix);
                    }
                }
            }
            TruncateFrom::Middle => {}
        }

        None
    }

    fn should_truncate_line_middle(
        &mut self,
        line: &str,
        truncate_width: Pixels,
        truncation_affix: &str,
    ) -> Option<(usize, usize)> {
        let affix_width = truncation_affix
            .chars()
            .map(|c| self.width_for_char(c))
            .fold(px(0.0), |width, char_width| width + char_width);
        let total_width = line
            .chars()
            .map(|c| self.width_for_char(c))
            .fold(px(0.0), |width, char_width| width + char_width);

        if total_width <= truncate_width {
            return None;
        }

        let content_budget = truncate_width - affix_width;
        if content_budget <= px(0.) {
            return Some((0, line.len()));
        }

        let front_budget = content_budget * (2.0 / 3.0);
        let back_budget = content_budget - front_budget;

        let mut front_width = px(0.);
        let mut front_end_ix = 0;
        for (ix, c) in line.char_indices() {
            let char_width = self.width_for_char(c);
            if front_width + char_width > front_budget {
                break;
            }
            front_width += char_width;
            front_end_ix = ix + c.len_utf8();
        }

        let mut back_width = px(0.);
        let mut back_start_ix = line.len();
        for (ix, c) in line.char_indices().rev() {
            let char_width = self.width_for_char(c);
            if back_width + char_width > back_budget {
                break;
            }
            back_width += char_width;
            back_start_ix = ix;
        }

        if front_end_ix >= back_start_ix {
            return Some((0, line.len()));
        }

        Some((front_end_ix, back_start_ix))
    }

    /// Truncate a line of text to the given width with this wrapper's font and font size.
    pub fn truncate_line<'a>(
        &mut self,
        line: SharedString,
        truncate_width: Pixels,
        truncation_affix: &str,
        runs: &'a [TextRun],
        truncate_from: TruncateFrom,
    ) -> (SharedString, Cow<'a, [TextRun]>) {
        if truncate_from == TruncateFrom::Middle {
            if let Some((front_end_ix, back_start_ix)) =
                self.should_truncate_line_middle(&line, truncate_width, truncation_affix)
            {
                let result = SharedString::from(format!(
                    "{}{truncation_affix}{}",
                    &line[..front_end_ix],
                    &line[back_start_ix..]
                ));
                let mut runs = runs.to_vec();
                update_runs_after_middle_truncation(
                    truncation_affix,
                    &mut runs,
                    front_end_ix,
                    back_start_ix,
                );
                return (result, Cow::Owned(runs));
            }

            return (line, Cow::Borrowed(runs));
        }

        if let Some(truncate_ix) =
            self.should_truncate_line(&line, truncate_width, truncation_affix, truncate_from)
        {
            let result = truncated_line_text(&line, truncate_ix, truncation_affix, truncate_from);
            let mut runs = runs.to_vec();
            update_runs_after_truncation(&result, truncation_affix, &mut runs, truncate_from);
            (result, Cow::Owned(runs))
        } else {
            (line, Cow::Borrowed(runs))
        }
    }

    /// Truncate text to fit within a given number of wrapped lines.
    pub fn truncate_wrapped_line<'a>(
        &mut self,
        text: SharedString,
        wrap_width: Pixels,
        max_lines: usize,
        truncation_affix: &str,
        runs: &'a [TextRun],
        truncate_from: TruncateFrom,
    ) -> (SharedString, Cow<'a, [TextRun]>) {
        if max_lines == 0 {
            return (SharedString::from(""), Cow::Owned(Vec::new()));
        }

        if truncate_from == TruncateFrom::Middle {
            return self.truncate_wrapped_line_middle(
                text,
                wrap_width,
                max_lines,
                truncation_affix,
                runs,
            );
        }

        if max_lines <= 1 {
            return self.truncate_single_wrapped_line(
                text,
                wrap_width,
                truncation_affix,
                runs,
                truncate_from,
            );
        }

        if truncate_from == TruncateFrom::Start {
            return self.truncate_wrapped_line_start(
                text,
                wrap_width,
                max_lines,
                truncation_affix,
                runs,
            );
        }

        let affix_width = truncation_affix
            .chars()
            .map(|c| self.width_for_char(c))
            .fold(px(0.), |width, char_width| width + char_width);

        let mut width = px(0.);
        let mut line = 0;
        let mut first_non_whitespace_ix = None;
        let mut indent = None;
        let mut last_candidate_ix = 0;
        let mut last_candidate_width = px(0.);
        let mut last_wrap_ix = 0;
        let mut prev_c = '\0';
        let mut truncate_ix = 0;

        for (ix, c) in text.char_indices() {
            if c == '\n' {
                if line >= max_lines - 1
                    && text
                        .get(ix + 1..)
                        .is_some_and(|suffix| !suffix.trim().is_empty())
                {
                    let result = SharedString::from(format!(
                        "{}{truncation_affix}",
                        trim_end_before_truncation_affix(&text[..truncate_ix], truncation_affix)
                    ));
                    let mut runs = runs.to_vec();
                    update_runs_after_truncation(
                        &result,
                        truncation_affix,
                        &mut runs,
                        TruncateFrom::End,
                    );
                    return (result, Cow::Owned(runs));
                }

                line += 1;
                width = px(0.);
                first_non_whitespace_ix = None;
                indent = None;
                last_candidate_ix = 0;
                last_candidate_width = px(0.);
                last_wrap_ix = ix + 1;
                prev_c = '\0';
                truncate_ix = ix + 1;
                continue;
            }

            let char_width = self.width_for_char(c);

            if Self::is_word_char(c) {
                if prev_c == ' ' && c != ' ' && first_non_whitespace_ix.is_some() {
                    last_candidate_ix = ix;
                    last_candidate_width = width;
                }
            } else if c != ' ' && first_non_whitespace_ix.is_some() {
                last_candidate_ix = ix;
                last_candidate_width = width;
            }

            if c != ' ' && first_non_whitespace_ix.is_none() {
                first_non_whitespace_ix = Some(ix);
            }

            width += char_width;

            if line < max_lines - 1 {
                if width > wrap_width && ix > last_wrap_ix {
                    if let (None, Some(first_non_whitespace_ix)) = (indent, first_non_whitespace_ix)
                    {
                        indent = Some(
                            Self::MAX_INDENT.min((first_non_whitespace_ix - last_wrap_ix) as u32),
                        );
                    }

                    if last_candidate_ix > 0 {
                        last_wrap_ix = last_candidate_ix;
                        width -= last_candidate_width;
                        last_candidate_ix = 0;
                    } else {
                        last_wrap_ix = ix;
                        width = char_width;
                    }

                    if let Some(indent) = indent {
                        width += self.width_for_char(' ') * indent as f32;
                    }

                    line += 1;
                    truncate_ix = last_wrap_ix;
                }
            } else {
                if width + affix_width <= wrap_width {
                    truncate_ix = ix + c.len_utf8();
                }

                if width > wrap_width {
                    let result = SharedString::from(format!(
                        "{}{truncation_affix}",
                        trim_end_before_truncation_affix(&text[..truncate_ix], truncation_affix)
                    ));
                    let mut runs = runs.to_vec();
                    update_runs_after_truncation(
                        &result,
                        truncation_affix,
                        &mut runs,
                        TruncateFrom::End,
                    );
                    return (result, Cow::Owned(runs));
                }
            }

            prev_c = c;
        }

        (text, Cow::Borrowed(runs))
    }

    fn truncate_wrapped_line_middle<'a>(
        &mut self,
        text: SharedString,
        wrap_width: Pixels,
        max_lines: usize,
        truncation_affix: &str,
        runs: &'a [TextRun],
    ) -> (SharedString, Cow<'a, [TextRun]>) {
        if self.wrapped_line_count(&text, wrap_width) <= max_lines {
            return (text, Cow::Borrowed(runs));
        }

        let mut boundaries = Vec::with_capacity(text.chars().count() + 1);
        boundaries.push(0);
        boundaries.extend(text.char_indices().map(|(ix, c)| ix + c.len_utf8()));
        let character_count = boundaries.len() - 1;

        let candidate = |retained_characters: usize| {
            let front_characters = retained_characters.saturating_mul(2).div_ceil(3);
            let back_characters = retained_characters - front_characters;
            let front_end_ix = boundaries[front_characters];
            let back_start_ix = boundaries[character_count - back_characters];
            let result = SharedString::from(format!(
                "{}{truncation_affix}{}",
                &text[..front_end_ix],
                &text[back_start_ix..]
            ));
            (result, front_end_ix, back_start_ix)
        };

        // The line-count check keeps hard newlines and word wrapping authoritative;
        // multiplying the one-line width by `max_lines` does not model either.
        let mut low = 0;
        let mut high = character_count.saturating_sub(1);
        while low < high {
            let mid = (low + high).div_ceil(2);
            let (result, _, _) = candidate(mid);
            if self.wrapped_line_count(&result, wrap_width) <= max_lines {
                low = mid;
            } else {
                high = mid - 1;
            }
        }

        let (result, front_end_ix, back_start_ix) = candidate(low);
        let mut runs = runs.to_vec();
        update_runs_after_middle_truncation(
            truncation_affix,
            &mut runs,
            front_end_ix,
            back_start_ix,
        );
        (result, Cow::Owned(runs))
    }

    fn truncate_single_wrapped_line<'a>(
        &mut self,
        text: SharedString,
        truncate_width: Pixels,
        truncation_affix: &str,
        runs: &'a [TextRun],
        truncate_from: TruncateFrom,
    ) -> (SharedString, Cow<'a, [TextRun]>) {
        let Some(newline_ix) = text.find('\n') else {
            return self.truncate_line(text, truncate_width, truncation_affix, runs, truncate_from);
        };

        let first_line = &text[..newline_ix];
        let (result, affix_inserted) = if !text[newline_ix + 1..].trim().is_empty() {
            (
                self.truncate_line_with_forced_affix(
                    first_line,
                    truncate_width,
                    truncation_affix,
                    truncate_from,
                ),
                !truncation_affix.is_empty(),
            )
        } else if let Some(truncate_ix) =
            self.should_truncate_line(first_line, truncate_width, truncation_affix, truncate_from)
        {
            (
                truncated_line_text(first_line, truncate_ix, truncation_affix, truncate_from),
                !truncation_affix.is_empty(),
            )
        } else {
            (SharedString::from(first_line), false)
        };

        let mut result_runs = runs_for_prefix(runs, newline_ix);
        if affix_inserted || result.len() < first_line.len() {
            let affix = if affix_inserted { truncation_affix } else { "" };
            update_runs_after_truncation(&result, affix, &mut result_runs, truncate_from);
            result_runs.retain(|run| run.len > 0);
        }

        if result_runs.is_empty()
            && !result.is_empty()
            && let Some(mut run) = runs.first().cloned()
        {
            run.len = result.len();
            result_runs.push(run);
        }

        (result, Cow::Owned(result_runs))
    }

    fn truncate_line_with_forced_affix(
        &mut self,
        line: &str,
        truncate_width: Pixels,
        truncation_affix: &str,
        truncate_from: TruncateFrom,
    ) -> SharedString {
        if truncation_affix.is_empty() {
            if let Some(truncate_ix) =
                self.should_truncate_line(line, truncate_width, truncation_affix, truncate_from)
            {
                truncated_line_text(line, truncate_ix, truncation_affix, truncate_from)
            } else {
                SharedString::from(line)
            }
        } else {
            let affix_width = truncation_affix
                .chars()
                .map(|c| self.width_for_char(c))
                .fold(px(0.), |width, char_width| width + char_width);

            match truncate_from {
                TruncateFrom::Start => {
                    let mut width = px(0.);
                    let mut truncate_ix = line.len();
                    for (ix, c) in line.char_indices().rev() {
                        let char_width = self.width_for_char(c);
                        if width + char_width + affix_width <= truncate_width {
                            width += char_width;
                            truncate_ix = ix;
                        } else {
                            break;
                        }
                    }
                    SharedString::from(format!("{truncation_affix}{}", &line[truncate_ix..]))
                }
                TruncateFrom::End => {
                    let mut width = px(0.);
                    let mut truncate_ix = 0;
                    for (ix, c) in line.char_indices() {
                        let char_width = self.width_for_char(c);
                        if width + char_width + affix_width <= truncate_width {
                            width += char_width;
                            truncate_ix = ix + c.len_utf8();
                        } else {
                            break;
                        }
                    }
                    SharedString::from(format!(
                        "{}{truncation_affix}",
                        trim_end_before_truncation_affix(&line[..truncate_ix], truncation_affix)
                    ))
                }
                TruncateFrom::Middle => {
                    unreachable!("middle truncation is handled before wrapped-line truncation")
                }
            }
        }
    }

    fn truncate_wrapped_line_start<'a>(
        &mut self,
        text: SharedString,
        wrap_width: Pixels,
        max_lines: usize,
        truncation_affix: &str,
        runs: &'a [TextRun],
    ) -> (SharedString, Cow<'a, [TextRun]>) {
        if self.wrapped_line_count(&text, wrap_width) <= max_lines {
            return (text, Cow::Borrowed(runs));
        }

        let mut candidates = text.char_indices().map(|(ix, _)| ix).collect::<Vec<_>>();
        candidates.push(text.len());

        let mut low = 0;
        let mut high = candidates.len() - 1;
        while low < high {
            let mid = (low + high) / 2;
            let candidate = candidates[mid];

            if self.wrapped_line_count_with_prefix(truncation_affix, &text[candidate..], wrap_width)
                <= max_lines
            {
                high = mid;
            } else {
                low = mid + 1;
            }
        }

        let result = SharedString::from(format!("{truncation_affix}{}", &text[candidates[low]..]));
        let mut runs = runs.to_vec();
        update_runs_after_truncation(&result, truncation_affix, &mut runs, TruncateFrom::Start);
        runs.retain(|run| run.len > 0);
        (result, Cow::Owned(runs))
    }

    fn wrapped_line_count(&mut self, text: &str, wrap_width: Pixels) -> usize {
        text.split_terminator('\n')
            .map(|line| {
                self.wrap_line(&[LineFragment::text(line)], wrap_width)
                    .count()
                    + 1
            })
            .sum()
    }

    fn wrapped_line_count_with_prefix(
        &mut self,
        prefix: &str,
        text: &str,
        wrap_width: Pixels,
    ) -> usize {
        let mut lines = text.split_terminator('\n');
        let Some(first_line) = lines.next() else {
            return 0;
        };

        let first_line_count = self
            .wrap_line(
                &[LineFragment::text(prefix), LineFragment::text(first_line)],
                wrap_width,
            )
            .count()
            + 1;

        first_line_count
            + lines
                .map(|line| {
                    self.wrap_line(&[LineFragment::text(line)], wrap_width)
                        .count()
                        + 1
                })
                .sum::<usize>()
    }

    /// Any character in this list should be treated as a word character,
    /// meaning it can be part of a word that should not be wrapped.
    pub(crate) fn is_word_char(c: char) -> bool {
        // ASCII alphanumeric characters, for English, numbers: `Hello123`, etc.
        c.is_ascii_alphanumeric() ||
        // Latin script in Unicode for French, German, Spanish, etc.
        // Latin-1 Supplement
        // https://en.wikipedia.org/wiki/Latin-1_Supplement
        matches!(c, '\u{00C0}'..='\u{00FF}') ||
        // Latin Extended-A
        // https://en.wikipedia.org/wiki/Latin_Extended-A
        matches!(c, '\u{0100}'..='\u{017F}') ||
        // Latin Extended-B
        // https://en.wikipedia.org/wiki/Latin_Extended-B
        matches!(c, '\u{0180}'..='\u{024F}') ||
        // Cyrillic for Russian, Ukrainian, etc.
        // https://en.wikipedia.org/wiki/Cyrillic_script_in_Unicode
        matches!(c, '\u{0400}'..='\u{04FF}') ||

        // Vietnamese (https://vietunicode.sourceforge.net/charset/)
        matches!(c, '\u{1E00}'..='\u{1EFF}') || // Latin Extended Additional
        matches!(c, '\u{0300}'..='\u{036F}') || // Combining Diacritical Marks

        // Bengali (https://en.wikipedia.org/wiki/Bengali_(Unicode_block))
        matches!(c, '\u{0980}'..='\u{09FF}') ||

        // Some other known special characters that should be treated as word characters,
        // e.g. `a-b`, `var_name`, `I'm`/`won't`, '@mention`, `#hashtag`, `100%`, `3.1415`,
        // `2^3`, `a~b`, `a=1`, `Self::new`, etc. Trailing punctuation like `,`, `.`, `:`, `;`
        // is included so it stays attached to the preceding word when wrapping.
        matches!(c, '-' | '_' | '.' | '\'' | '\u{2018}' | '\u{2019}' | '$' | '%' | '@' | '#' | '^' | '~' | ',' | '=' | ':' | ';') ||
        // `⋯` character is special used in Zed, to keep this at the end of the line.
        matches!(c, '⋯') ||

        // Non-breaking glue characters
        matches!(c, '\u{202F}' | '\u{00A0}' | '\u{2011}')
    }

    #[inline(always)]
    fn width_for_char(&mut self, c: char) -> Pixels {
        if (c as u32) < 128 {
            if let Some(cached_width) = self.cached_ascii_char_widths[c as usize] {
                cached_width
            } else {
                let width = self
                    .text_system
                    .layout_width(self.font_id, self.font_size, c);
                self.cached_ascii_char_widths[c as usize] = Some(width);
                width
            }
        } else if let Some(cached_width) = self.cached_other_char_widths.get(&c) {
            *cached_width
        } else {
            let width = self
                .text_system
                .layout_width(self.font_id, self.font_size, c);
            self.cached_other_char_widths.insert(c, width);
            width
        }
    }
}

fn truncated_line_text(
    line: &str,
    truncate_ix: usize,
    truncation_affix: &str,
    truncate_from: TruncateFrom,
) -> SharedString {
    match truncate_from {
        TruncateFrom::Start => SharedString::from(format!(
            "{truncation_affix}{}",
            &line[line.ceil_char_boundary(truncate_ix + 1)..]
        )),
        TruncateFrom::End => SharedString::from(format!(
            "{}{truncation_affix}",
            trim_end_before_truncation_affix(&line[..truncate_ix], truncation_affix)
        )),
        TruncateFrom::Middle => unreachable!("middle truncation is handled by truncate_line"),
    }
}

fn trim_end_before_truncation_affix<'a>(text: &'a str, truncation_affix: &str) -> &'a str {
    if truncation_affix.is_empty() {
        text
    } else {
        text.trim_end_matches(|c: char| c.is_whitespace() || is_truncation_boundary_punctuation(c))
    }
}

fn is_truncation_boundary_punctuation(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::ConnectorPunctuation
            | GeneralCategory::DashPunctuation
            | GeneralCategory::OpenPunctuation
            | GeneralCategory::ClosePunctuation
            | GeneralCategory::InitialPunctuation
            | GeneralCategory::FinalPunctuation
            | GeneralCategory::OtherPunctuation
    )
}

fn runs_for_prefix(runs: &[TextRun], prefix_len: usize) -> Vec<TextRun> {
    let mut prefix_runs = Vec::new();
    let mut remaining_len = prefix_len;

    for run in runs {
        if remaining_len == 0 {
            break;
        }

        let mut run = run.clone();
        if run.len > remaining_len {
            run.len = remaining_len;
            prefix_runs.push(run);
            break;
        }

        remaining_len -= run.len;
        prefix_runs.push(run);
    }

    prefix_runs
}

fn update_runs_after_truncation(
    result: &str,
    ellipsis: &str,
    runs: &mut Vec<TextRun>,
    truncate_from: TruncateFrom,
) {
    let mut truncate_at = result.len() - ellipsis.len();
    match truncate_from {
        TruncateFrom::Start => {
            let mut first_retained_run_ix = None;
            for (run_index, run) in runs.iter_mut().enumerate().rev() {
                if run.len <= truncate_at {
                    truncate_at -= run.len;
                    first_retained_run_ix = Some(run_index);
                } else {
                    run.len = truncate_at + ellipsis.len();
                    runs.splice(..run_index, std::iter::empty());
                    runs.retain(|run| run.len > 0);
                    return;
                }
            }
            if let Some(run_index) = first_retained_run_ix {
                runs[run_index].len += ellipsis.len();
                runs.splice(..run_index, std::iter::empty());
            }
        }
        TruncateFrom::End => {
            let mut last_retained_run_ix = None;
            for (run_index, run) in runs.iter_mut().enumerate() {
                if run.len <= truncate_at {
                    truncate_at -= run.len;
                    last_retained_run_ix = Some(run_index);
                } else {
                    run.len = truncate_at + ellipsis.len();
                    runs.truncate(run_index + 1);
                    runs.retain(|run| run.len > 0);
                    return;
                }
            }
            if let Some(run_index) = last_retained_run_ix {
                runs[run_index].len += ellipsis.len();
                runs.truncate(run_index + 1);
            }
        }
        TruncateFrom::Middle => {
            unreachable!("middle truncation updates runs through its dedicated helper")
        }
    }
    runs.retain(|run| run.len > 0);
}

fn update_runs_after_middle_truncation(
    ellipsis: &str,
    runs: &mut Vec<TextRun>,
    front_end_ix: usize,
    back_start_ix: usize,
) {
    let original_runs = std::mem::take(runs);
    let mut result_runs = Vec::with_capacity(original_runs.len());

    let mut front_remaining = front_end_ix;
    let mut front_done = false;
    for run in &original_runs {
        if front_done {
            break;
        }
        if run.len <= front_remaining {
            result_runs.push(run.clone());
            front_remaining -= run.len;
        } else {
            let mut partial = run.clone();
            partial.len = front_remaining + ellipsis.len();
            result_runs.push(partial);
            front_done = true;
        }
    }
    if !front_done {
        if let Some(last) = result_runs.last_mut() {
            last.len += ellipsis.len();
        } else if let Some(first) = original_runs.first() {
            let mut affix_run = first.clone();
            affix_run.len = ellipsis.len();
            result_runs.push(affix_run);
        }
    }

    let mut byte_pos = 0;
    for run in &original_runs {
        let run_end = byte_pos + run.len;
        if run_end > back_start_ix {
            if byte_pos < back_start_ix {
                let mut partial = run.clone();
                partial.len = run_end - back_start_ix;
                result_runs.push(partial);
            } else {
                result_runs.push(run.clone());
            }
        }
        byte_pos = run_end;
    }

    result_runs.retain(|run| run.len > 0);
    *runs = result_runs;
}

/// A fragment of a line that can be wrapped.
pub enum LineFragment<'a> {
    /// A text fragment consisting of characters.
    Text {
        /// The text content of the fragment.
        text: &'a str,
    },
    /// A non-text element with a fixed width.
    Element {
        /// The width of the element in pixels.
        width: Pixels,
        /// The UTF-8 encoded length of the element.
        len_utf8: usize,
    },
}

impl<'a> LineFragment<'a> {
    /// Creates a new text fragment from the given text.
    pub fn text(text: &'a str) -> Self {
        LineFragment::Text { text }
    }

    /// Creates a new non-text element with the given width and UTF-8 encoded length.
    pub fn element(width: Pixels, len_utf8: usize) -> Self {
        LineFragment::Element { width, len_utf8 }
    }

    fn wrap_boundary_candidates(&self) -> impl Iterator<Item = WrapBoundaryCandidate> {
        let text = match self {
            LineFragment::Text { text } => text,
            LineFragment::Element { .. } => "\0",
        };
        text.chars().map(move |character| {
            if let LineFragment::Element { width, len_utf8 } = self {
                WrapBoundaryCandidate::Element {
                    width: *width,
                    len_utf8: *len_utf8,
                }
            } else {
                WrapBoundaryCandidate::Char { character }
            }
        })
    }
}

enum WrapBoundaryCandidate {
    Char { character: char },
    Element { width: Pixels, len_utf8: usize },
}

impl WrapBoundaryCandidate {
    pub fn len_utf8(&self) -> usize {
        match self {
            WrapBoundaryCandidate::Char { character } => character.len_utf8(),
            WrapBoundaryCandidate::Element { len_utf8: len, .. } => *len,
        }
    }
}

/// A boundary between two lines of text.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Boundary {
    /// The index of the last character in a line
    pub ix: usize,
    /// The indent of the next line.
    pub next_indent: u32,
}

impl Boundary {
    fn new(ix: usize, next_indent: u32) -> Self {
        Self { ix, next_indent }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Font, FontFeatures, FontStyle, FontWeight, TestAppContext, TestDispatcher, font};
    #[cfg(target_os = "macos")]
    use crate::{TextRun, WindowTextSystem, WrapBoundary};

    fn build_wrapper() -> LineWrapper {
        let dispatcher = TestDispatcher::new(0);
        let cx = TestAppContext::build(dispatcher, None);
        let id = cx.text_system().resolve_font(&font(".ZedMono"));
        LineWrapper::new(id, px(16.), cx.text_system().clone())
    }

    fn generate_test_runs(input_run_len: &[usize]) -> Vec<TextRun> {
        input_run_len
            .iter()
            .map(|run_len| TextRun {
                len: *run_len,
                font: Font {
                    family: "Dummy".into(),
                    features: FontFeatures::default(),
                    fallbacks: None,
                    weight: FontWeight::default(),
                    style: FontStyle::Normal,
                },
                ..Default::default()
            })
            .collect()
    }

    fn wrapped_line_count(wrapper: &mut LineWrapper, text: &str, wrap_width: Pixels) -> usize {
        text.split_terminator('\n')
            .map(|line| {
                wrapper
                    .wrap_line(&[LineFragment::text(line)], wrap_width)
                    .count()
                    + 1
            })
            .sum()
    }

    #[test]
    fn test_wrap_line() {
        let mut wrapper = build_wrapper();

        assert_eq!(
            wrapper
                .wrap_line(&[LineFragment::text("aa bbb cccc ddddd eeee")], px(72.))
                .collect::<Vec<_>>(),
            &[
                Boundary::new(7, 0),
                Boundary::new(12, 0),
                Boundary::new(18, 0)
            ],
        );

        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("a\u{202f}b\u{00a0}c\u{2011}d e")],
                    px(72.0)
                )
                .collect::<Vec<_>>(),
            &[Boundary::new(12, 0)],
        );
        assert_eq!(
            wrapper
                .wrap_line(&[LineFragment::text("aaa aaaaaaaaaaaaaaaaaa")], px(72.0))
                .collect::<Vec<_>>(),
            &[
                Boundary::new(4, 0),
                Boundary::new(11, 0),
                Boundary::new(18, 0)
            ],
        );
        assert_eq!(
            wrapper
                .wrap_line(&[LineFragment::text("     aaaaaaa")], px(72.))
                .collect::<Vec<_>>(),
            &[
                Boundary::new(7, 5),
                Boundary::new(9, 5),
                Boundary::new(11, 5),
            ]
        );
        assert_eq!(
            wrapper
                .wrap_line(
                    &[LineFragment::text("                            ")],
                    px(72.)
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(7, 0),
                Boundary::new(14, 0),
                Boundary::new(21, 0)
            ]
        );
        assert_eq!(
            wrapper
                .wrap_line(&[LineFragment::text("          aaaaaaaaaaaaaa")], px(72.))
                .collect::<Vec<_>>(),
            &[
                Boundary::new(7, 0),
                Boundary::new(14, 3),
                Boundary::new(18, 3),
                Boundary::new(22, 3),
            ]
        );

        // Test wrapping multiple text fragments
        assert_eq!(
            wrapper
                .wrap_line(
                    &[
                        LineFragment::text("aa bbb "),
                        LineFragment::text("cccc ddddd eeee")
                    ],
                    px(72.)
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(7, 0),
                Boundary::new(12, 0),
                Boundary::new(18, 0)
            ],
        );

        // Test wrapping with a mix of text and element fragments
        assert_eq!(
            wrapper
                .wrap_line(
                    &[
                        LineFragment::text("aa "),
                        LineFragment::element(px(20.), 1),
                        LineFragment::text(" bbb "),
                        LineFragment::element(px(30.), 1),
                        LineFragment::text(" cccc")
                    ],
                    px(72.)
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(5, 0),
                Boundary::new(9, 0),
                Boundary::new(11, 0)
            ],
        );

        // Test with element at the beginning and text afterward
        assert_eq!(
            wrapper
                .wrap_line(
                    &[
                        LineFragment::element(px(50.), 1),
                        LineFragment::text(" aaaa bbbb cccc dddd")
                    ],
                    px(72.)
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(2, 0),
                Boundary::new(7, 0),
                Boundary::new(12, 0),
                Boundary::new(17, 0)
            ],
        );

        // Test with a large element that forces wrapping by itself
        assert_eq!(
            wrapper
                .wrap_line(
                    &[
                        LineFragment::text("short text "),
                        LineFragment::element(px(100.), 1),
                        LineFragment::text(" more text")
                    ],
                    px(72.)
                )
                .collect::<Vec<_>>(),
            &[
                Boundary::new(6, 0),
                Boundary::new(11, 0),
                Boundary::new(12, 0),
                Boundary::new(18, 0)
            ],
        );
    }

    #[test]
    fn test_truncate_line_end() {
        let mut wrapper = build_wrapper();

        fn perform_test(
            wrapper: &mut LineWrapper,
            text: &'static str,
            expected: &'static str,
            ellipsis: &str,
            line_width: Pixels,
        ) {
            let dummy_run_lens = vec![text.len()];
            let dummy_runs = generate_test_runs(&dummy_run_lens);
            let (result, dummy_runs) = wrapper.truncate_line(
                text.into(),
                line_width,
                ellipsis,
                &dummy_runs,
                TruncateFrom::End,
            );
            assert_eq!(result, expected);
            assert_eq!(dummy_runs.first().unwrap().len, result.len());
        }

        perform_test(
            &mut wrapper,
            "aa bbb cccc ddddd eeee ffff gggg",
            "aa bbb cccc ddddd eeee",
            "",
            px(220.),
        );
        perform_test(
            &mut wrapper,
            "aa bbb cccc ddddd eeee ffff gggg",
            "aa bbb cccc ddddd eee…",
            "…",
            px(220.),
        );
        perform_test(
            &mut wrapper,
            "aa bbb cccc ddddd eeee ffff gggg",
            "aa bbb cccc dddd......",
            "......",
            px(220.),
        );
        perform_test(
            &mut wrapper,
            "aa bbb cccc 🦀🦀🦀🦀🦀 eeee ffff gggg",
            "aa bbb cccc 🦀🦀🦀🦀…",
            "…",
            px(220.),
        );
        perform_test(&mut wrapper, "hello world", "hello…", "…", px(70.));
        perform_test(&mut wrapper, "hello, world", "hello…", "…", px(70.));
        perform_test(&mut wrapper, "hello, world", "hello, ", "", px(70.));
    }

    #[test]
    fn test_truncate_line_end_trims_unicode_punctuation_before_affix() {
        let mut wrapper = build_wrapper();

        for text in ["hello— world", "hello، world"] {
            let dummy_runs = generate_test_runs(&[text.len()]);
            let line_width = text
                .chars()
                .take(6)
                .chain("…".chars())
                .map(|c| wrapper.width_for_char(c))
                .fold(px(0.), |width, char_width| width + char_width)
                + px(1.);

            let (result, result_runs) =
                wrapper.truncate_line(text.into(), line_width, "…", &dummy_runs, TruncateFrom::End);

            assert_eq!(result, "hello…");
            assert_eq!(
                result_runs.iter().map(|run| run.len).sum::<usize>(),
                result.len()
            );
        }
    }

    #[test]
    fn test_truncate_line_end_preserves_unicode_punctuation_without_affix() {
        let mut wrapper = build_wrapper();
        let text = "hello— world";
        let dummy_runs = generate_test_runs(&[text.len()]);
        let line_width = "hello—"
            .chars()
            .map(|c| wrapper.width_for_char(c))
            .fold(px(0.), |width, char_width| width + char_width)
            + px(1.);

        let (result, result_runs) =
            wrapper.truncate_line(text.into(), line_width, "", &dummy_runs, TruncateFrom::End);

        assert_eq!(result, "hello—");
        assert_eq!(
            result_runs.iter().map(|run| run.len).sum::<usize>(),
            result.len()
        );
    }

    #[test]
    fn test_truncate_line_start() {
        let mut wrapper = build_wrapper();

        #[track_caller]
        fn perform_test(
            wrapper: &mut LineWrapper,
            text: &'static str,
            expected: &'static str,
            ellipsis: &str,
        ) {
            let dummy_run_lens = vec![text.len()];
            let dummy_runs = generate_test_runs(&dummy_run_lens);
            let (result, dummy_runs) = wrapper.truncate_line(
                text.into(),
                px(220.),
                ellipsis,
                &dummy_runs,
                TruncateFrom::Start,
            );
            assert_eq!(result, expected);
            assert_eq!(dummy_runs.first().unwrap().len, result.len());
        }

        perform_test(
            &mut wrapper,
            "aaaa bbbb cccc ddddd eeee fff gg",
            "cccc ddddd eeee fff gg",
            "",
        );
        perform_test(
            &mut wrapper,
            "aaaa bbbb cccc ddddd eeee fff gg",
            "…ccc ddddd eeee fff gg",
            "…",
        );
        perform_test(
            &mut wrapper,
            "aaaa bbbb cccc ddddd eeee fff gg",
            "......dddd eeee fff gg",
            "......",
        );
        perform_test(
            &mut wrapper,
            "aaaa bbbb cccc 🦀🦀🦀🦀🦀 eeee fff gg",
            "…🦀🦀🦀🦀 eeee fff gg",
            "…",
        );
    }

    #[test]
    fn test_truncate_multiple_runs_end() {
        let mut wrapper = build_wrapper();

        fn perform_test(
            wrapper: &mut LineWrapper,
            text: &'static str,
            expected: &str,
            run_lens: &[usize],
            result_run_len: &[usize],
            line_width: Pixels,
        ) {
            let dummy_runs = generate_test_runs(run_lens);
            let (result, dummy_runs) =
                wrapper.truncate_line(text.into(), line_width, "…", &dummy_runs, TruncateFrom::End);
            assert_eq!(result, expected);
            for (run, result_len) in dummy_runs.iter().zip(result_run_len) {
                assert_eq!(run.len, *result_len);
            }
        }
        // Case 0: Normal
        // Text: abcdefghijkl
        // Runs: Run0 { len: 12, ... }
        //
        // Truncate res: abcd… (truncate_at = 4)
        // Run res: Run0 { string: abcd…, len: 7, ... }
        perform_test(&mut wrapper, "abcdefghijkl", "abcd…", &[12], &[7], px(50.));
        // Case 1: Drop some runs
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: abcdef… (truncate_at = 6)
        // Runs res: Run0 { string: abcd, len: 4, ... }, Run1 { string: ef…, len:
        // 5, ... }
        perform_test(
            &mut wrapper,
            "abcdefghijkl",
            "abcdef…",
            &[4, 4, 4],
            &[4, 5],
            px(70.),
        );
        // Case 2: Truncate at start of some run
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: abcdefgh… (truncate_at = 8)
        // Runs res: Run0 { string: abcd, len: 4, ... }, Run1 { string: efgh, len:
        // 4, ... }, Run2 { string: …, len: 3, ... }
        perform_test(
            &mut wrapper,
            "abcdefghijkl",
            "abcdefgh…",
            &[4, 4, 4],
            &[4, 4, 3],
            px(90.),
        );
    }

    #[test]
    fn test_truncate_multiple_runs_start() {
        let mut wrapper = build_wrapper();

        #[track_caller]
        fn perform_test(
            wrapper: &mut LineWrapper,
            text: &'static str,
            expected: &str,
            run_lens: &[usize],
            result_run_len: &[usize],
            line_width: Pixels,
        ) {
            let dummy_runs = generate_test_runs(run_lens);
            let (result, dummy_runs) = wrapper.truncate_line(
                text.into(),
                line_width,
                "…",
                &dummy_runs,
                TruncateFrom::Start,
            );
            assert_eq!(result, expected);
            for (run, result_len) in dummy_runs.iter().zip(result_run_len) {
                assert_eq!(run.len, *result_len);
            }
        }
        // Case 0: Normal
        // Text: abcdefghijkl
        // Runs: Run0 { len: 12, ... }
        //
        // Truncate res: …ijkl (truncate_at = 9)
        // Run res: Run0 { string: …ijkl, len: 7, ... }
        perform_test(&mut wrapper, "abcdefghijkl", "…ijkl", &[12], &[7], px(50.));
        // Case 1: Drop some runs
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: …ghijkl (truncate_at = 7)
        // Runs res: Run0 { string: …gh, len: 5, ... }, Run1 { string: ijkl, len:
        // 4, ... }
        perform_test(
            &mut wrapper,
            "abcdefghijkl",
            "…ghijkl",
            &[4, 4, 4],
            &[5, 4],
            px(70.),
        );
        // Case 2: Truncate at start of some run
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: abcdefgh… (truncate_at = 3)
        // Runs res: Run0 { string: …, len: 3, ... }, Run1 { string: efgh, len:
        // 4, ... }, Run2 { string: ijkl, len: 4, ... }
        perform_test(
            &mut wrapper,
            "abcdefghijkl",
            "…efghijkl",
            &[4, 4, 4],
            &[3, 4, 4],
            px(90.),
        );
    }

    #[test]
    fn test_update_run_after_truncation_end() {
        fn perform_test(result: &str, run_lens: &[usize], result_run_lens: &[usize]) {
            let mut dummy_runs = generate_test_runs(run_lens);
            update_runs_after_truncation(result, "…", &mut dummy_runs, TruncateFrom::End);
            for (run, result_len) in dummy_runs.iter().zip(result_run_lens) {
                assert_eq!(run.len, *result_len);
            }
        }
        // Case 0: Normal
        // Text: abcdefghijkl
        // Runs: Run0 { len: 12, ... }
        //
        // Truncate res: abcd… (truncate_at = 4)
        // Run res: Run0 { string: abcd…, len: 7, ... }
        perform_test("abcd…", &[12], &[7]);
        // Case 1: Drop some runs
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: abcdef… (truncate_at = 6)
        // Runs res: Run0 { string: abcd, len: 4, ... }, Run1 { string: ef…, len:
        // 5, ... }
        perform_test("abcdef…", &[4, 4, 4], &[4, 5]);
        // Case 2: Truncate at start of some run
        // Text: abcdefghijkl
        // Runs: Run0 { len: 4, ... }, Run1 { len: 4, ... }, Run2 { len: 4, ... }
        //
        // Truncate res: abcdefgh… (truncate_at = 8)
        // Runs res: Run0 { string: abcd, len: 4, ... }, Run1 { string: efgh, len:
        // 4, ... }, Run2 { string: …, len: 3, ... }
        perform_test("abcdefgh…", &[4, 4, 4], &[4, 4, 3]);
    }

    #[test]
    fn test_update_runs_after_truncation_drops_empty_boundary_runs() {
        let mut end_runs = generate_test_runs(&[4, 4, 4]);
        update_runs_after_truncation("abcdefgh", "", &mut end_runs, TruncateFrom::End);
        assert_eq!(
            end_runs.iter().map(|run| run.len).collect::<Vec<_>>(),
            vec![4, 4]
        );

        let mut start_runs = generate_test_runs(&[4, 4, 4]);
        update_runs_after_truncation("efghijkl", "", &mut start_runs, TruncateFrom::Start);
        assert_eq!(
            start_runs.iter().map(|run| run.len).collect::<Vec<_>>(),
            vec![4, 4]
        );
    }

    #[test]
    fn test_multiline_truncation_fits_within_wrapped_lines() {
        let mut wrapper = build_wrapper();
        let text = "aa bbbbbb cccccc dddddd eeee ffff";
        let wrap_width = px(72.);
        let max_lines = 2;
        let runs = generate_test_runs(&[text.len()]);

        let (truncated, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "…",
            &runs,
            TruncateFrom::End,
        );

        let wrapped_line_count = wrapper
            .wrap_line(&[LineFragment::text(&truncated)], wrap_width)
            .count()
            + 1;
        assert!(
            wrapped_line_count <= max_lines,
            "{truncated:?} wrapped into {wrapped_line_count} lines"
        );
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn test_wrapped_truncation_zero_lines_returns_empty_text_and_runs() {
        let mut wrapper = build_wrapper();
        let text = "aa bbbbbb cccccc";
        let runs = generate_test_runs(&[2, 1, text.len() - 3]);

        let (result, result_runs) =
            wrapper.truncate_wrapped_line(text.into(), px(72.), 0, "…", &runs, TruncateFrom::End);

        assert_eq!(result, "");
        assert!(result_runs.is_empty());
    }

    #[test]
    fn test_multiline_truncation_no_truncation_needed() {
        let mut wrapper = build_wrapper();
        let text = "aa bbb cccccc";
        let runs = generate_test_runs(&[text.len()]);

        let (result, _) =
            wrapper.truncate_wrapped_line(text.into(), px(72.), 2, "…", &runs, TruncateFrom::End);

        assert_eq!(result, text);
    }

    #[test]
    fn test_multiline_truncation_three_lines() {
        let mut wrapper = build_wrapper();
        let text = "aa bbb cccc ddddd eeee ffff gggg hhhh iiii jjjj";
        let wrap_width = px(72.);
        let max_lines = 3;
        let runs = generate_test_runs(&[text.len()]);

        let (truncated, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "…",
            &runs,
            TruncateFrom::End,
        );

        let wrapped_line_count = wrapper
            .wrap_line(&[LineFragment::text(&truncated)], wrap_width)
            .count()
            + 1;
        assert!(
            wrapped_line_count <= max_lines,
            "{truncated:?} wrapped into {wrapped_line_count} lines"
        );
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn test_multiline_truncation_with_newlines() {
        let mut wrapper = build_wrapper();
        let text = "hello\nworld foo bar baz";
        let wrap_width = px(72.);
        let runs = generate_test_runs(&[text.len()]);

        let (truncated, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            2,
            "…",
            &runs,
            TruncateFrom::End,
        );

        let mut lines = truncated.splitn(2, '\n');
        assert_eq!(lines.next(), Some("hello"));
        let second_line = lines.next().expect("newline should be preserved");
        let second_line_width = second_line
            .chars()
            .map(|c| wrapper.width_for_char(c))
            .fold(px(0.), |width, char_width| width + char_width);
        assert!(second_line_width <= wrap_width);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn test_multiline_truncation_newline_on_last_line() {
        let mut wrapper = build_wrapper();
        let text = "hello\nworld\nmore";
        let runs = generate_test_runs(&[text.len()]);

        let (truncated, _) =
            wrapper.truncate_wrapped_line(text.into(), px(72.), 2, "…", &runs, TruncateFrom::End);

        assert_eq!(truncated.split('\n').next(), Some("hello"));
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn test_truncate_line_middle() {
        let mut wrapper = build_wrapper();

        let short_text = "hello world";
        let runs = generate_test_runs(&[short_text.len()]);
        let (result, result_runs) = wrapper.truncate_line(
            short_text.into(),
            px(10000.),
            "…",
            &runs,
            TruncateFrom::Middle,
        );
        assert_eq!(result.as_ref(), short_text);
        assert!(matches!(result_runs, Cow::Borrowed(_)));

        let long_text = "abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz";
        let runs = generate_test_runs(&[long_text.len()]);
        let (result, _) =
            wrapper.truncate_line(long_text.into(), px(100.), "…", &runs, TruncateFrom::Middle);
        assert!(result.contains('…'));
        assert!(result.chars().count() < long_text.chars().count());
        assert_eq!(result.chars().next(), long_text.chars().next());
        assert_eq!(result.chars().last(), long_text.chars().last());

        let text = "abcdef";
        let runs = generate_test_runs(&[text.len()]);
        let (result, result_runs) =
            wrapper.truncate_line(text.into(), px(1.), "…", &runs, TruncateFrom::Middle);
        assert_eq!(result.as_ref(), "…");
        assert_eq!(
            result_runs.iter().map(|run| run.len).sum::<usize>(),
            "…".len()
        );

        let runs = generate_test_runs(&[20, 20, long_text.len() - 40]);
        let (result, result_runs) =
            wrapper.truncate_line(long_text.into(), px(100.), "…", &runs, TruncateFrom::Middle);
        assert_eq!(
            result_runs.iter().map(|run| run.len).sum::<usize>(),
            result.len()
        );
    }

    #[test]
    fn test_wrapped_middle_truncation_respects_max_lines() {
        let mut wrapper = build_wrapper();
        let text = "beginning alpha beta gamma\nmiddle delta epsilon zeta\nending eta theta omega";
        let wrap_width = px(72.);
        let max_lines = 2;
        let runs = generate_test_runs(&[10, 15, text.len() - 25]);

        let (result, result_runs) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "…",
            &runs,
            TruncateFrom::Middle,
        );

        assert_eq!(result.chars().next(), text.chars().next());
        assert_eq!(result.chars().last(), text.chars().last());
        assert!(result.contains('…'));
        assert!(wrapped_line_count(&mut wrapper, &result, wrap_width) <= max_lines);
        assert_eq!(
            result_runs.iter().map(|run| run.len).sum::<usize>(),
            result.len()
        );
        assert!(result_runs.iter().all(|run| run.len > 0));
    }

    #[test]
    fn test_middle_truncation_drops_empty_boundary_runs() {
        let mut runs = generate_test_runs(&[4, 4, 4]);

        update_runs_after_middle_truncation("", &mut runs, 4, 8);

        assert_eq!(
            runs.iter().map(|run| run.len).collect::<Vec<_>>(),
            vec![4, 4]
        );
    }

    #[test]
    fn test_multiline_truncation_trailing_newline() {
        let mut wrapper = build_wrapper();
        let text = "hello\nworld\n";
        let runs = generate_test_runs(&[text.len()]);

        let (result, _) =
            wrapper.truncate_wrapped_line(text.into(), px(72.), 2, "…", &runs, TruncateFrom::End);

        assert_eq!(result, text);
    }

    #[test]
    fn test_multiline_start_truncation_fits_within_wrapped_lines() {
        let mut wrapper = build_wrapper();
        let text = "aa bbbbbb cccccc dddddd eeee ffff";
        let wrap_width = px(72.);
        let max_lines = 2;
        let runs = generate_test_runs(&[text.len()]);

        let (truncated, _) = wrapper.truncate_wrapped_line(
            text.into(),
            wrap_width,
            max_lines,
            "…",
            &runs,
            TruncateFrom::Start,
        );

        assert!(truncated.starts_with('…'));
        let line_count = wrapped_line_count(&mut wrapper, &truncated, wrap_width);
        assert!(
            line_count <= max_lines,
            "{truncated:?} wrapped into {line_count} lines"
        );
    }

    #[test]
    fn test_multiline_start_truncation_no_truncation_needed() {
        let mut wrapper = build_wrapper();
        let text = "aa bbb cccccc";
        let runs = generate_test_runs(&[text.len()]);

        let (result, result_runs) =
            wrapper.truncate_wrapped_line(text.into(), px(72.), 2, "…", &runs, TruncateFrom::Start);

        assert_eq!(result, text);
        assert!(matches!(result_runs, Cow::Borrowed(_)));
    }

    #[test]
    fn test_multiline_start_truncation_trailing_newline() {
        let mut wrapper = build_wrapper();
        let text = "hello\nworld\n";
        let runs = generate_test_runs(&[text.len()]);

        let (result, result_runs) = wrapper.truncate_wrapped_line(
            text.into(),
            px(500.),
            2,
            "…",
            &runs,
            TruncateFrom::Start,
        );

        assert_eq!(result, text);
        assert!(matches!(result_runs, Cow::Borrowed(_)));
    }

    #[test]
    fn test_multiline_start_truncation_updates_runs() {
        let mut wrapper = build_wrapper();
        let text = "abcdefghijklmnopqrst";
        let runs = generate_test_runs(&[5, 5, 5, 5]);

        let (result, result_runs) =
            wrapper.truncate_wrapped_line(text.into(), px(72.), 2, "…", &runs, TruncateFrom::Start);

        assert_eq!(result, "…hijklmnopqrst");
        let result_runs = result_runs.into_owned();
        assert_eq!(
            result_runs.iter().map(|run| run.len).collect::<Vec<_>>(),
            vec![6, 5, 5]
        );
        assert_eq!(
            result_runs.iter().map(|run| run.len).sum::<usize>(),
            result.len()
        );
        assert!(result_runs.iter().all(|run| run.len > 0));
    }

    #[test]
    fn test_one_line_wrapped_truncation_stops_at_hard_newline() {
        let mut wrapper = build_wrapper();
        let text = "hello\nworld";
        let runs = generate_test_runs(&[5, 1, 5]);

        let (result, result_runs) =
            wrapper.truncate_wrapped_line(text.into(), px(500.), 1, "…", &runs, TruncateFrom::End);

        assert_eq!(result, "hello…");
        assert_eq!(
            result_runs.iter().map(|run| run.len).sum::<usize>(),
            result.len()
        );
        assert!(result_runs.iter().all(|run| run.len > 0));
    }

    #[test]
    fn test_one_line_wrapped_truncation_leading_newline_covers_affix_run() {
        let mut wrapper = build_wrapper();
        let text = "\nworld";
        let runs = generate_test_runs(&[1, 5]);

        let (result, result_runs) =
            wrapper.truncate_wrapped_line(text.into(), px(500.), 1, "…", &runs, TruncateFrom::End);

        assert_eq!(result, "…");
        assert_eq!(
            result_runs.iter().map(|run| run.len).sum::<usize>(),
            result.len()
        );
        assert!(result_runs.iter().all(|run| run.len > 0));
    }

    #[test]
    fn test_is_word_char() {
        #[track_caller]
        fn assert_word(word: &str) {
            for c in word.chars() {
                assert!(
                    LineWrapper::is_word_char(c),
                    "assertion failed for '{}' (unicode 0x{:x})",
                    c,
                    c as u32
                );
            }
        }

        #[track_caller]
        fn assert_not_word(word: &str) {
            let found = word.chars().any(|c| !LineWrapper::is_word_char(c));
            assert!(found, "assertion failed for '{}'", word);
        }

        assert_word("Hello123");
        assert_word("non-English");
        assert_word("var_name");
        assert_word("123456");
        assert_word("3.1415");
        assert_word("10^2");
        assert_word("1~2");
        assert_word("100%");
        assert_word("@mention");
        assert_word("#hashtag");
        assert_word("$variable");
        assert_word("a=1");
        assert_word("Self::is_word_char");
        assert_word("more⋯");
        assert_word("won\u{2019}t");
        assert_word("\u{2018}twas");

        // Space
        assert_not_word("foo bar");

        // URL case
        assert_word("github.com");
        assert_not_word("zed-industries/zed");
        assert_not_word("zed-industries\\zed");
        assert_not_word("a=1&b=2");
        assert_not_word("foo?b=2");

        // Latin-1 Supplement
        assert_word("ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏ");
        // Latin Extended-A
        assert_word("ĀāĂăĄąĆćĈĉĊċČčĎď");
        // Latin Extended-B
        assert_word("ƀƁƂƃƄƅƆƇƈƉƊƋƌƍƎƏ");
        // Cyrillic
        assert_word("АБВГДЕЖЗИЙКЛМНОП");
        // Vietnamese (https://github.com/zed-industries/zed/issues/23245)
        assert_word("ThậmchíđếnkhithuachạychúngcònnhẫntâmgiếtnốtsốđôngtùchínhtrịởYênBáivàCaoBằng");
        // Bengali
        assert_word("গিয়েছিলেন");
        assert_word("ছেলে");
        assert_word("হচ্ছিল");

        // non-word characters
        assert_not_word("你好");
        assert_not_word("안녕하세요");
        assert_not_word("こんにちは");
        assert_not_word("😀😁😂");
        assert_not_word("()[]{}<>");

        assert_word("\u{202f}");
        assert_word("\u{00a0}");
        assert_word("\u{2011}");
    }

    // For compatibility with the test macro
    #[cfg(target_os = "macos")]
    use crate as gpui;

    // These seem to vary wildly based on the text system.
    #[cfg(target_os = "macos")]
    #[crate::test]
    fn test_wrap_shaped_line(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let text_system = WindowTextSystem::new(cx.text_system().clone());

            let normal = TextRun {
                len: 0,
                font: font("Helvetica"),
                color: Default::default(),
                underline: Default::default(),
                ..Default::default()
            };
            let bold = TextRun {
                len: 0,
                font: font("Helvetica").bold(),
                ..Default::default()
            };

            let text = "aa bbb cccc ddddd eeee".into();
            let lines = text_system
                .shape_text(
                    text,
                    px(16.),
                    &[
                        normal.with_len(4),
                        bold.with_len(5),
                        normal.with_len(6),
                        bold.with_len(1),
                        normal.with_len(7),
                    ],
                    Some(px(72.)),
                    None,
                )
                .unwrap();

            assert_eq!(
                lines[0].layout.wrap_boundaries(),
                &[
                    WrapBoundary {
                        run_ix: 0,
                        glyph_ix: 7
                    },
                    WrapBoundary {
                        run_ix: 0,
                        glyph_ix: 12
                    },
                    WrapBoundary {
                        run_ix: 0,
                        glyph_ix: 18
                    }
                ],
            );
        });
    }
}
