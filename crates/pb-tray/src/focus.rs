//! Keyboard focus order: Tab and Shift+Tab move through a window's focusable
//! items (by id) and wrap at either end.

/// The id after `current` in `order` (before it when `back`), wrapping. With
/// nothing focused, Tab goes to the first item and Shift+Tab to the last; an
/// id no longer in `order` restarts the same way. None when `order` is empty.
pub fn next(order: &[u32], current: Option<u32>, back: bool) -> Option<u32> {
    let n = order.len();
    if n == 0 {
        return None;
    }
    let i = match current.and_then(|c| order.iter().position(|&id| id == c)) {
        Some(i) if back => (i + n - 1) % n,
        Some(i) => (i + 1) % n,
        None if back => n - 1,
        None => 0,
    };
    Some(order[i])
}

/// An arrow-style key inside a group of items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Prev,
    Next,
    First,
    Last,
}

/// Where `mv` goes from `current` in `order`: no wrapping (arrows stop at
/// the ends), and from nothing, Next / First start at the first item and
/// Prev / Last at the last.
pub fn step(order: &[u32], current: Option<u32>, mv: Move) -> Option<u32> {
    let n = order.len();
    if n == 0 {
        return None;
    }
    let at = current.and_then(|c| order.iter().position(|&id| id == c));
    let i = match (mv, at) {
        (Move::First, _) | (Move::Next, None) => 0,
        (Move::Last, _) | (Move::Prev, None) => n - 1,
        (Move::Next, Some(i)) => (i + 1).min(n - 1),
        (Move::Prev, Some(i)) => i.saturating_sub(1),
    };
    Some(order[i])
}

/// Left / Right on a collapsible header: `Some(open)` is the state it asks
/// for, None when the key means nothing there (already in that state).
pub fn header_key(open: bool, right: bool) -> Option<bool> {
    (open != right).then_some(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_stop_at_the_ends_and_home_end_jump() {
        let o = [4, 5, 6];
        assert_eq!(step(&o, Some(6), Move::Next), Some(6));
        assert_eq!(step(&o, Some(4), Move::Prev), Some(4));
        assert_eq!(step(&o, Some(5), Move::Next), Some(6));
        assert_eq!(step(&o, Some(6), Move::First), Some(4));
        assert_eq!(step(&o, Some(4), Move::Last), Some(6));
        assert_eq!(step(&o, None, Move::Next), Some(4));
        assert_eq!(step(&o, None, Move::Prev), Some(6));
        assert_eq!(step(&[], Some(1), Move::Next), None);
    }

    #[test]
    fn left_right_collapse_and_expand_a_header() {
        assert_eq!(header_key(true, false), Some(false));
        assert_eq!(header_key(false, true), Some(true));
        assert_eq!(header_key(true, true), None);
        assert_eq!(header_key(false, false), None);
    }

    #[test]
    fn tab_wraps_forward_and_back() {
        assert_eq!(next(&[1, 2, 3], Some(3), false), Some(1));
        assert_eq!(next(&[1, 2, 3], Some(1), true), Some(3));
        assert_eq!(next(&[], None, false), None);
    }

    #[test]
    fn steps_and_starts_from_either_end() {
        assert_eq!(next(&[1, 2, 3], Some(1), false), Some(2));
        assert_eq!(next(&[1, 2, 3], Some(3), true), Some(2));
        assert_eq!(next(&[1, 2, 3], None, false), Some(1));
        assert_eq!(next(&[1, 2, 3], None, true), Some(3));
        // The focused item went away (a page re-laid out): start again.
        assert_eq!(next(&[1, 2, 3], Some(9), false), Some(1));
    }
}
