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

#[cfg(test)]
mod tests {
    use super::*;

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
