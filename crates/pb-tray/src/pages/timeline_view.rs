//! Which Timeline days are open. Pure state: the rule for the first view,
//! the user's per-day choices (kept for the session by the window), toggle,
//! and collapse / expand all. Days are identified by their local day number.

use std::collections::BTreeMap;

/// More groups than this and only the newest starts open.
pub const OPEN_ALL_UP_TO: usize = 3;

/// The first view of group `index` (0 = newest, normally Today) among
/// `groups`: all open when there are few days, else only the newest.
pub fn default_expanded(index: usize, groups: usize) -> bool {
    groups <= OPEN_ALL_UP_TO || index == 0
}

/// The user's choices on top of the default rule. A day with no choice
/// follows the rule, so new days appearing keep the page tidy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expansion {
    choice: BTreeMap<i64, bool>,
}

impl Expansion {
    /// Whether `day`, group `index` of `days` (newest first), is open.
    pub fn is_open(&self, days: &[i64], index: usize) -> bool {
        match days.get(index) {
            Some(day) => self
                .choice
                .get(day)
                .copied()
                .unwrap_or_else(|| default_expanded(index, days.len())),
            None => false,
        }
    }

    /// Flips one day.
    pub fn toggle(&mut self, days: &[i64], day: i64) {
        let Some(index) = days.iter().position(|&d| d == day) else {
            return;
        };
        let now = self.is_open(days, index);
        self.choice.insert(day, !now);
    }

    /// Opens or closes every day.
    pub fn set_all(&mut self, days: &[i64], open: bool) {
        for &d in days {
            self.choice.insert(d, open);
        }
    }

    /// True when every day is open (the control then offers "Collapse all").
    pub fn all_open(&self, days: &[i64]) -> bool {
        (0..days.len()).all(|k| self.is_open(days, k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(n: usize) -> Vec<i64> {
        (0..n as i64).map(|k| 20_000 - k).collect()
    }

    #[test]
    fn few_groups_start_open_many_start_with_only_the_newest() {
        for n in 1..=3 {
            let d = days(n);
            let e = Expansion::default();
            assert!((0..n).all(|k| e.is_open(&d, k)), "{n} groups");
        }
        let d = days(5);
        let e = Expansion::default();
        assert!(e.is_open(&d, 0));
        assert!((1..5).all(|k| !e.is_open(&d, k)));
        assert!(default_expanded(0, 9) && !default_expanded(1, 9));
        assert!(default_expanded(2, 3) && !default_expanded(2, 4));
    }

    #[test]
    fn toggle_flips_the_effective_state_and_is_remembered() {
        let d = days(5);
        let mut e = Expansion::default();
        e.toggle(&d, d[2]);
        assert!(e.is_open(&d, 2));
        assert!(!e.is_open(&d, 3));
        e.toggle(&d, d[2]);
        assert!(!e.is_open(&d, 2));
        e.toggle(&d, d[0]);
        assert!(!e.is_open(&d, 0));
        // A new day arriving on top: the old choices follow their day.
        let mut shifted = vec![20_001];
        shifted.extend(&d);
        assert!(e.is_open(&shifted, 0), "the new newest follows the rule");
        assert!(
            !e.is_open(&shifted, 1),
            "Today collapsed by the user stays so"
        );
    }

    #[test]
    fn collapse_all_and_expand_all() {
        let d = days(5);
        let mut e = Expansion::default();
        assert!(!e.all_open(&d));
        e.set_all(&d, true);
        assert!(e.all_open(&d));
        assert!((0..5).all(|k| e.is_open(&d, k)));
        e.set_all(&d, false);
        assert!(!e.all_open(&d));
        assert!((0..5).all(|k| !e.is_open(&d, k)));
        e.toggle(&d, d[4]);
        assert!(e.is_open(&d, 4) && !e.is_open(&d, 3));
        // With few days everything is open to begin with.
        assert!(Expansion::default().all_open(&days(2)));
        assert!(Expansion::default().all_open(&[]));
    }
}
