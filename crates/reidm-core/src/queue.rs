//! The queue's order: which waiting download starts next, and moving downloads in it.
//!
//! Every download has a place ([`Item::order`]); of those queued, the lowest starts first.
//! Moving one changes the places of only the downloads that actually move, so a move in a long
//! list counts as a change to a few rows, not to all of them.

use crate::{Manager, Status, ORDER_BASE};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Where to move downloads in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Move {
    Top,
    Up,
    Down,
    Bottom,
}

impl Move {
    pub fn from_name(s: &str) -> Option<Move> {
        match s.trim().to_ascii_lowercase().as_str() {
            "top" => Some(Move::Top),
            "up" => Some(Move::Up),
            "down" => Some(Move::Down),
            "bottom" => Some(Move::Bottom),
            _ => None,
        }
    }
}

/// The new order of `seq` (ids in queue order) after moving the `chosen` ones one place up or
/// down. A block of chosen downloads moves together and keeps its own order; one already at the
/// end it is moving towards stays, and so does the rest of the block behind it.
fn step(seq: &mut [u64], chosen: &HashSet<u64>, up: bool) {
    let n = seq.len();
    let idx: Vec<usize> = if up { (1..n).collect() } else { (0..n.saturating_sub(1)).rev().collect() };
    for i in idx {
        let j = if up { i - 1 } else { i + 1 };
        if chosen.contains(&seq[i]) && !chosen.contains(&seq[j]) {
            seq.swap(i, j);
        }
    }
}

impl Manager {
    /// Move downloads in the queue. Up and down move each past the next download still to do
    /// (finished ones, which never wait in the queue, are stepped over); top and bottom put
    /// them before or after every other. Several move as a block, in their own order.
    pub fn move_in_queue(&self, ids: &[u64], to: Move) -> Result<()> {
        let chosen: HashSet<u64> = ids.iter().copied().collect();
        {
            let mut db = self.0.db.lock().unwrap();
            if let Some(id) = ids.iter().find(|id| !db.items.contains_key(id)) {
                return Err(anyhow!("there is no download #{id}"));
            }
            let mut all: Vec<(u64, u64)> = db.items.values().map(|i| (i.order, i.id)).collect();
            all.sort_unstable();
            let changes: Vec<(u64, u64)> = match to {
                Move::Top | Move::Bottom => {
                    let picked: Vec<u64> = all.iter().filter(|(_, id)| chosen.contains(id)).map(|(_, id)| *id).collect();
                    let k = picked.len() as u64;
                    let (lo, hi) = (all.first().map_or(ORDER_BASE, |f| f.0), all.last().map_or(ORDER_BASE, |l| l.0));
                    if to == Move::Top && lo <= k {
                        // Out of room below: number the queue again (after billions of moves).
                        let mut seq: Vec<u64> = picked.clone();
                        seq.extend(all.iter().map(|(_, id)| *id).filter(|id| !chosen.contains(id)));
                        seq.iter().enumerate().map(|(n, id)| (*id, ORDER_BASE + n as u64)).collect()
                    } else {
                        let first = if to == Move::Top { lo - k } else { hi + 1 };
                        picked.iter().enumerate().map(|(n, id)| (*id, first + n as u64)).collect()
                    }
                }
                Move::Up | Move::Down => {
                    // Only downloads still to do take part, plus the chosen ones themselves.
                    let part: Vec<(u64, u64)> = all.into_iter().filter(|(_, id)| chosen.contains(id) || db.items.get(id).is_some_and(|i| i.status != Status::Done)).collect();
                    let mut seq: Vec<u64> = part.iter().map(|(_, id)| *id).collect();
                    step(&mut seq, &chosen, to == Move::Up);
                    // Each position keeps its number; the downloads change places.
                    seq.iter().zip(&part).filter(|(id, (_, was))| *id != was).map(|(id, (order, _))| (*id, *order)).collect()
                }
            };
            for (id, order) in changes {
                if let Some(it) = db.items.get_mut(&id).filter(|it| it.order != order) {
                    it.order = order;
                }
            }
            let top = db.items.values().map(|i| i.order).max().unwrap_or(ORDER_BASE);
            db.next_order = db.next_order.max(top + 1);
        }
        self.save();
        Ok(())
    }

    /// The ids of all downloads, in queue order.
    pub fn queue_order(&self) -> Vec<u64> {
        let db = self.0.db.lock().unwrap();
        let mut v: Vec<(u64, u64)> = db.items.values().map(|i| (i.order, i.id)).collect();
        v.sort_unstable();
        v.into_iter().map(|(_, id)| id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(seq: &[u64], chosen: &[u64], up: bool) -> Vec<u64> {
        let mut s = seq.to_vec();
        step(&mut s, &chosen.iter().copied().collect(), up);
        s
    }

    #[test]
    fn a_block_moves_together_and_keeps_its_order() {
        assert_eq!(moved(&[1, 2, 3, 4, 5], &[3, 4], true), [1, 3, 4, 2, 5]);
        assert_eq!(moved(&[1, 2, 3, 4, 5], &[3, 4], false), [1, 2, 5, 3, 4]);
        // Already at the top: stays, and what follows it in the block stays behind it.
        assert_eq!(moved(&[1, 2, 3, 4, 5], &[1, 2], true), [1, 2, 3, 4, 5]);
        assert_eq!(moved(&[1, 2, 3, 4, 5], &[1, 3], true), [1, 3, 2, 4, 5]);
        assert_eq!(moved(&[1, 2, 3, 4, 5], &[5], false), [1, 2, 3, 4, 5]);
        assert_eq!(moved(&[1, 2, 3, 4, 5], &[2, 4], false), [1, 3, 2, 5, 4]);
        assert_eq!(moved(&[], &[1], true), Vec::<u64>::new());
    }
}
