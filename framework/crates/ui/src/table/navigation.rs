pub(super) fn first_target(count: usize) -> Option<usize> {
    (count > 0).then_some(0)
}

pub(super) fn last_target(count: usize) -> Option<usize> {
    count.checked_sub(1)
}

pub(super) fn previous_target(
    current: Option<usize>,
    count: usize,
    loop_selection: bool,
) -> Option<usize> {
    let last = last_target(count)?;
    Some(match current {
        Some(current) if current > 0 => current.saturating_sub(1),
        Some(_) if loop_selection => last,
        Some(current) => current,
        None => 0,
    })
}

pub(super) fn next_target(
    current: Option<usize>,
    count: usize,
    loop_selection: bool,
) -> Option<usize> {
    let last = last_target(count)?;
    Some(match current {
        Some(current) if current < last => current + 1,
        Some(_) if loop_selection => 0,
        Some(current) => current,
        None => 0,
    })
}

pub(super) fn page_up_target(current: Option<usize>, step: usize, count: usize) -> Option<usize> {
    first_target(count).map(|_| current.unwrap_or(0).saturating_sub(step))
}

pub(super) fn page_down_target(current: Option<usize>, step: usize, count: usize) -> Option<usize> {
    let last = last_target(count)?;
    Some((current.unwrap_or(0) + step).min(last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_and_last_targets_handle_empty_counts() {
        assert_eq!(first_target(0), None);
        assert_eq!(last_target(0), None);
        assert_eq!(first_target(3), Some(0));
        assert_eq!(last_target(3), Some(2));
    }

    #[test]
    fn previous_target_respects_looping_and_bootstrap() {
        assert_eq!(previous_target(None, 4, false), Some(0));
        assert_eq!(previous_target(Some(2), 4, false), Some(1));
        assert_eq!(previous_target(Some(0), 4, false), Some(0));
        assert_eq!(previous_target(Some(0), 4, true), Some(3));
        assert_eq!(previous_target(Some(0), 0, true), None);
    }

    #[test]
    fn next_target_respects_looping_and_bootstrap() {
        assert_eq!(next_target(None, 4, false), Some(0));
        assert_eq!(next_target(Some(1), 4, false), Some(2));
        assert_eq!(next_target(Some(3), 4, false), Some(3));
        assert_eq!(next_target(Some(3), 4, true), Some(0));
        assert_eq!(next_target(Some(0), 0, true), None);
    }

    #[test]
    fn page_targets_clamp_to_table_bounds() {
        assert_eq!(page_up_target(None, 5, 10), Some(0));
        assert_eq!(page_up_target(Some(7), 5, 10), Some(2));
        assert_eq!(page_up_target(Some(2), 5, 10), Some(0));
        assert_eq!(page_up_target(Some(2), 5, 0), None);

        assert_eq!(page_down_target(None, 5, 10), Some(5));
        assert_eq!(page_down_target(Some(2), 5, 10), Some(7));
        assert_eq!(page_down_target(Some(8), 5, 10), Some(9));
        assert_eq!(page_down_target(Some(0), 5, 0), None);
    }
}
