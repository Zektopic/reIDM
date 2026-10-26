//! Lists of downloads in and out: batch patterns of addresses.
//!
//! ## Batch patterns
//!
//! An address with one `*` and a range — `img*.jpg`, 1 to 20, zero-padded to 3 digits gives
//! `img001.jpg` … `img020.jpg`; a letter range does the same with `a` … `z`.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// What replaces the `*` in a batch pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Sequence {
    /// `from..=to`, zero-padded to `width` digits (0 or 1 means no padding).
    Numbers {
        from: u64,
        to: u64,
        #[serde(default)]
        width: usize,
    },
    /// `from..=to` over ASCII letters of one case, e.g. `a`..`z`.
    Letters { from: char, to: char },
}

/// The most addresses one pattern may produce. A typo like `1` to `1000000` should be an error,
/// not a million queued downloads.
pub const MAX_PATTERN_ITEMS: usize = 10_000;

/// The widest zero-padding a number may have: the largest number has 20 digits, so wider only
/// costs memory.
pub const MAX_PATTERN_WIDTH: usize = 20;

/// Expand a pattern with exactly one `*`.
pub fn expand_pattern(pattern: &str, seq: &Sequence) -> Result<Vec<String>> {
    let pattern = pattern.trim();
    let stars = pattern.matches('*').count();
    if stars != 1 {
        bail!("the address needs exactly one * to replace (it has {stars})");
    }
    let (head, tail) = pattern.split_once('*').expect("one star");
    let items: Vec<String> = match *seq {
        Sequence::Numbers { from, to, width } => {
            let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
            // Checked: `0` to `u64::MAX` is one more than a u64 holds, and an unchecked count
            // wrapped to 0, passed the size check and then never finished counting.
            let n = (hi - lo).checked_add(1);
            if n.is_none_or(|n| n > MAX_PATTERN_ITEMS as u64) {
                let n = n.map_or_else(|| "18446744073709551616".to_string(), |n| n.to_string());
                bail!("{lo} to {hi} is {n} addresses, more than the {MAX_PATTERN_ITEMS} a pattern may produce; use a smaller range");
            }
            if width > MAX_PATTERN_WIDTH {
                bail!("numbers can be padded to at most {MAX_PATTERN_WIDTH} digits (asked for {width})");
            }
            (lo..=hi).map(|i| format!("{i:0width$}", width = width.max(1))).collect()
        }
        Sequence::Letters { from, to } => {
            let same_case = (from.is_ascii_lowercase() && to.is_ascii_lowercase()) || (from.is_ascii_uppercase() && to.is_ascii_uppercase());
            if !same_case {
                bail!("a letter range must be a..z or A..Z in one case (got {from:?}..{to:?})");
            }
            let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
            (lo..=hi).map(|c| c.to_string()).collect()
        }
    };
    Ok(items.into_iter().map(|x| format!("{head}{x}{tail}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_patterns_pad_and_order() {
        let v = expand_pattern("https://x/img*.jpg", &Sequence::Numbers { from: 8, to: 11, width: 3 }).unwrap();
        assert_eq!(v, ["https://x/img008.jpg", "https://x/img009.jpg", "https://x/img010.jpg", "https://x/img011.jpg"]);
        // A reversed range is the same range.
        let r = expand_pattern("https://x/*.jpg", &Sequence::Numbers { from: 3, to: 1, width: 0 }).unwrap();
        assert_eq!(r, ["https://x/1.jpg", "https://x/2.jpg", "https://x/3.jpg"]);
    }

    #[test]
    fn letter_patterns() {
        let v = expand_pattern("https://x/part-*.rar", &Sequence::Letters { from: 'a', to: 'c' }).unwrap();
        assert_eq!(v, ["https://x/part-a.rar", "https://x/part-b.rar", "https://x/part-c.rar"]);
        assert!(expand_pattern("https://x/*", &Sequence::Letters { from: 'a', to: 'C' }).is_err());
    }

    #[test]
    fn patterns_need_one_star_and_a_sane_size() {
        assert!(expand_pattern("https://x/a.jpg", &Sequence::Numbers { from: 1, to: 2, width: 0 }).is_err());
        assert!(expand_pattern("https://x/*/*.jpg", &Sequence::Numbers { from: 1, to: 2, width: 0 }).is_err());
        assert!(expand_pattern("https://x/*.jpg", &Sequence::Numbers { from: 0, to: 1_000_000, width: 0 }).is_err());
    }

    #[test]
    fn huge_ranges_and_widths_are_refused_at_once() {
        // The whole of u64 used to wrap the count to zero and then expand without end.
        let e = expand_pattern("https://x/*", &Sequence::Numbers { from: 0, to: u64::MAX, width: 0 }).unwrap_err();
        assert!(e.to_string().contains("more than the 10000"), "{e}");
        assert!(expand_pattern("https://x/*", &Sequence::Numbers { from: u64::MAX, to: 1, width: 0 }).is_err());
        assert!(expand_pattern("https://x/*", &Sequence::Numbers { from: 1, to: 2, width: usize::MAX }).is_err());
        // The boundaries themselves still work.
        assert_eq!(expand_pattern("https://x/*", &Sequence::Numbers { from: u64::MAX, to: u64::MAX, width: 20 }).unwrap(), ["https://x/18446744073709551615"]);
        assert_eq!(expand_pattern("https://x/*", &Sequence::Numbers { from: 1, to: 10_000, width: 0 }).unwrap().len(), 10_000);
    }
}
