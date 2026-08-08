//! Fuzzy matching implementations for the Command Palette.

use super::types::{CommandMatcher, CommandPaletteItem, CommandPaletteMatch};
use fuzzy_matcher::FuzzyMatcher as _;
use fuzzy_matcher::skim::SkimMatcherV2;
use nucleo::Utf32Str;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};

/// Nucleo-based fuzzy matcher (default).
///
/// Provides fast, async-friendly fuzzy matching with Unicode support.
#[derive(Clone, Default)]
pub struct NucleoMatcher;

impl NucleoMatcher {
    pub fn new() -> Self {
        Self
    }

    fn match_text(&self, query: &str, text: &str) -> Option<(i64, Vec<(usize, usize)>)> {
        if query.is_empty() {
            return Some((0, Vec::new()));
        }

        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let text_utf32: Vec<char> = text.chars().collect();
        let haystack = Utf32Str::Unicode(&text_utf32);

        let mut indices = Vec::new();
        let score = pattern.indices(
            haystack,
            &mut nucleo::Matcher::new(nucleo::Config::DEFAULT),
            &mut indices,
        )?;

        // Convert char indices to byte offset ranges
        let ranges = indices_to_ranges(&indices, text);

        Some((score as i64, ranges))
    }
}

impl CommandMatcher for NucleoMatcher {
    fn match_item(&self, query: &str, item: &CommandPaletteItem) -> Option<CommandPaletteMatch> {
        if query.is_empty() {
            return Some(CommandPaletteMatch::new(0));
        }

        // Try matching against title first
        let title_match = self.match_text(query, &item.title);

        // Try matching against subtitle
        let subtitle_match = item
            .subtitle
            .as_ref()
            .and_then(|s| self.match_text(query, s));

        // Try matching against keywords
        let keyword_match = item
            .keywords
            .iter()
            .filter_map(|k| self.match_text(query, k))
            .max_by_key(|(score, _)| *score);

        // Try matching against category
        let category_match = self.match_text(query, &item.category);

        // Combine scores and use the best match
        let mut best_score = None;
        let mut title_ranges = Vec::new();
        let mut subtitle_ranges = Vec::new();

        if let Some((score, ranges)) = title_match {
            best_score = Some(score + 1000); // Boost title matches
            title_ranges = ranges;
        }

        if let Some((score, ranges)) = subtitle_match {
            let adjusted = score + 500; // Boost subtitle matches
            if best_score.map_or(true, |s| adjusted > s) {
                best_score = Some(adjusted);
                subtitle_ranges = ranges;
            }
        }

        if let Some((score, _)) = keyword_match {
            let adjusted = score + 200; // Moderate boost for keyword matches
            if best_score.map_or(true, |s| adjusted > s) {
                best_score = Some(adjusted);
            }
        }

        if let Some((score, _)) = category_match {
            // Category match alone shouldn't be enough, but boost if other matches exist
            if best_score.is_some() {
                best_score = best_score.map(|s| s + score / 10);
            }
        }

        best_score.map(|score| {
            CommandPaletteMatch::new(score)
                .with_title_ranges(title_ranges)
                .with_subtitle_ranges(subtitle_ranges)
        })
    }
}

/// FuzzyMatcher-based matcher using SkimMatcherV2.
pub struct FuzzyMatcherWrapper {
    matcher: SkimMatcherV2,
}

impl Clone for FuzzyMatcherWrapper {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl Default for FuzzyMatcherWrapper {
    fn default() -> Self {
        Self::new()
    }
}

impl FuzzyMatcherWrapper {
    pub fn new() -> Self {
        Self {
            matcher: SkimMatcherV2::default(),
        }
    }

    fn match_text(&self, query: &str, text: &str) -> Option<(i64, Vec<(usize, usize)>)> {
        if query.is_empty() {
            return Some((0, Vec::new()));
        }

        self.matcher
            .fuzzy_indices(text, query)
            .map(|(score, indices)| {
                // fuzzy_matcher returns char indices, convert to byte offset ranges
                let ranges = indices_to_ranges_usize(&indices, text);
                (score, ranges)
            })
    }
}

impl CommandMatcher for FuzzyMatcherWrapper {
    fn match_item(&self, query: &str, item: &CommandPaletteItem) -> Option<CommandPaletteMatch> {
        if query.is_empty() {
            return Some(CommandPaletteMatch::new(0));
        }

        // Try matching against title first
        let title_match = self.match_text(query, &item.title);

        // Try matching against subtitle
        let subtitle_match = item
            .subtitle
            .as_ref()
            .and_then(|s| self.match_text(query, s));

        // Try matching against keywords
        let keyword_match = item
            .keywords
            .iter()
            .filter_map(|k| self.match_text(query, k))
            .max_by_key(|(score, _)| *score);

        // Combine scores and use the best match
        let mut best_score = None;
        let mut title_ranges = Vec::new();
        let mut subtitle_ranges = Vec::new();

        if let Some((score, ranges)) = title_match {
            best_score = Some(score + 1000); // Boost title matches
            title_ranges = ranges;
        }

        if let Some((score, ranges)) = subtitle_match {
            let adjusted = score + 500; // Boost subtitle matches
            if best_score.map_or(true, |s| adjusted > s) {
                best_score = Some(adjusted);
                subtitle_ranges = ranges;
            }
        }

        if let Some((score, _)) = keyword_match {
            let adjusted = score + 200; // Moderate boost for keyword matches
            if best_score.map_or(true, |s| adjusted > s) {
                best_score = Some(adjusted);
            }
        }

        best_score.map(|score| {
            CommandPaletteMatch::new(score)
                .with_title_ranges(title_ranges)
                .with_subtitle_ranges(subtitle_ranges)
        })
    }
}

/// Convert nucleo u32 char indices to byte offset ranges.
///
/// Nucleo returns character indices, but Rust strings use byte offsets.
/// This function converts character index ranges to byte offset ranges.
fn indices_to_ranges(indices: &[u32], text: &str) -> Vec<(usize, usize)> {
    if indices.is_empty() {
        return Vec::new();
    }

    // Build a map from char index to byte offset
    let char_to_byte: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let text_len = text.len();

    // First, convert char indices to consecutive char ranges
    let mut char_ranges = Vec::new();
    let mut start = indices[0] as usize;
    let mut end = start + 1;

    for &idx in &indices[1..] {
        let idx = idx as usize;
        if idx == end {
            end = idx + 1;
        } else {
            char_ranges.push((start, end));
            start = idx;
            end = idx + 1;
        }
    }
    char_ranges.push((start, end));

    // Convert char ranges to byte ranges
    char_ranges
        .into_iter()
        .filter_map(|(char_start, char_end)| {
            let byte_start = char_to_byte.get(char_start).copied()?;
            // For end, we need the byte offset of the character at char_end,
            // or the end of the string if char_end is past the last char
            let byte_end = if char_end < char_to_byte.len() {
                char_to_byte[char_end]
            } else {
                text_len
            };
            Some((byte_start, byte_end))
        })
        .collect()
}

/// Convert usize char indices to byte offset ranges.
///
/// fuzzy_matcher returns character indices, but Rust strings use byte offsets.
/// This function converts character index ranges to byte offset ranges.
fn indices_to_ranges_usize(indices: &[usize], text: &str) -> Vec<(usize, usize)> {
    if indices.is_empty() {
        return Vec::new();
    }

    // Build a map from char index to byte offset
    let char_to_byte: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let text_len = text.len();

    // First, convert char indices to consecutive char ranges
    let mut char_ranges = Vec::new();
    let mut start = indices[0];
    let mut end = start + 1;

    for &idx in &indices[1..] {
        if idx == end {
            end = idx + 1;
        } else {
            char_ranges.push((start, end));
            start = idx;
            end = idx + 1;
        }
    }
    char_ranges.push((start, end));

    // Convert char ranges to byte ranges
    char_ranges
        .into_iter()
        .filter_map(|(char_start, char_end)| {
            let byte_start = char_to_byte.get(char_start).copied()?;
            let byte_end = if char_end < char_to_byte.len() {
                char_to_byte[char_end]
            } else {
                text_len
            };
            Some((byte_start, byte_end))
        })
        .collect()
}
