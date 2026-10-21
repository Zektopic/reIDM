//! The order downloads wait in: the queue starts the waiting download with the lowest `order`
//! (the service keeps it). The window moves the selection to the top, up, down or to the bottom,
//! and shows each unfinished download's place.
//!
//! Only unfinished downloads take part: moving a finished one, or past one, would change
//! nothing anyone can see.

use crate::model::{Item, Status};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum To {
    Top,
    Up,
    Down,
    Bottom,
}

impl To {
    pub fn wire(self) -> &'static str {
        match self {
            To::Top => "top",
            To::Up => "up",
            To::Down => "down",
            To::Bottom => "bottom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            To::Top => "Move to the top",
            To::Up => "Move up",
            To::Down => "Move down",
            To::Bottom => "Move to the bottom",
        }
    }
}

/// The unfinished downloads in queue order, when the service keeps one.
pub fn queue<'a>(items: impl Iterator<Item = &'a Item>) -> Vec<u64> {
    let mut q: Vec<(u64, u64)> = items.filter(|i| i.status != Status::Done).filter_map(|i| Some((i.order?, i.id))).collect();
    q.sort_unstable();
    q.into_iter().map(|(_, id)| id).collect()
}

/// Each queued download's place, from 1.
pub fn places(queue: &[u64]) -> HashMap<u64, usize> {
    queue.iter().enumerate().map(|(i, id)| (*id, i + 1)).collect()
}

/// The queue after a move. Top and bottom take the selection as a block, in its own order; up
/// and down move each selected download one place past its unselected neighbour, so a block
/// moves as one and keeps its order, and a selection already at the end stays put.
pub fn moved(queue: &[u64], selected: &HashSet<u64>, to: To) -> Vec<u64> {
    let (sel, rest): (Vec<u64>, Vec<u64>) = queue.iter().partition(|id| selected.contains(id));
    match to {
        To::Top => sel.into_iter().chain(rest).collect(),
        To::Bottom => rest.into_iter().chain(sel).collect(),
        To::Up => {
            let mut q = queue.to_vec();
            for i in 1..q.len() {
                if selected.contains(&q[i]) && !selected.contains(&q[i - 1]) {
                    q.swap(i, i - 1);
                }
            }
            q
        }
        To::Down => {
            let mut q = queue.to_vec();
            for i in (0..q.len().saturating_sub(1)).rev() {
                if selected.contains(&q[i]) && !selected.contains(&q[i + 1]) {
                    q.swap(i, i + 1);
                }
            }
            q
        }
    }
}

/// Whether a move would change anything (the menu greys out the ones that wouldn't).
pub fn can_move(queue: &[u64], selected: &HashSet<u64>, to: To) -> bool {
    queue.iter().any(|id| selected.contains(id)) && moved(queue, selected, to) != queue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::tests::row;

    fn sel(ids: &[u64]) -> HashSet<u64> {
        ids.iter().copied().collect()
    }

    #[test]
    fn moves_keep_the_selection_s_own_order() {
        let q = [1, 2, 3, 4, 5, 6];
        assert_eq!(moved(&q, &sel(&[4, 2]), To::Top), [2, 4, 1, 3, 5, 6]);
        assert_eq!(moved(&q, &sel(&[4, 2]), To::Bottom), [1, 3, 5, 6, 2, 4]);
        assert_eq!(moved(&q, &sel(&[3, 4]), To::Up), [1, 3, 4, 2, 5, 6], "a block moves as one");
        assert_eq!(moved(&q, &sel(&[3, 4]), To::Down), [1, 2, 5, 3, 4, 6]);
        assert_eq!(moved(&q, &sel(&[1, 2, 5]), To::Up), [1, 2, 3, 5, 4, 6], "the part already at the top stays; the rest moves one place");
        assert_eq!(moved(&q, &sel(&[2, 6]), To::Down), [1, 3, 2, 4, 5, 6]);
    }

    #[test]
    fn a_move_that_changes_nothing_is_not_offered() {
        let q = [1, 2, 3];
        assert!(!can_move(&q, &sel(&[1]), To::Up) && !can_move(&q, &sel(&[1]), To::Top));
        assert!(!can_move(&q, &sel(&[2, 3]), To::Bottom));
        assert!(can_move(&q, &sel(&[1]), To::Down));
        assert!(!can_move(&q, &sel(&[9]), To::Top), "not in the queue (finished)");
    }

    #[test]
    fn the_queue_is_the_unfinished_downloads_by_order() {
        let mut items = [row(1, "a", "Queued", 0), row(2, "b", "Done", 1000), row(3, "c", "Paused", 0), row(4, "d", "Running", 5)];
        for (it, o) in items.iter_mut().zip([30, 10, 20, 40]) {
            it.order = Some(o);
        }
        let q = queue(items.iter());
        assert_eq!(q, [3, 1, 4]);
        assert_eq!(places(&q)[&1], 2);
        items[0].order = None;
        assert_eq!(queue(items.iter()), [3, 4], "an older service keeps no order");
    }
}
