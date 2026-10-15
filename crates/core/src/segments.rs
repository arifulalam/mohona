//! Cutting a file into byte ranges and handing them to workers.
//!
//! The [`Scheduler`] is the one place that knows which bytes belong to which
//! worker. Workers ask it for permission before writing ([`Scheduler::reserve`]),
//! and idle workers ask it for more work ([`Scheduler::steal`]), which cuts the
//! biggest unfinished segment in half. Everything happens under one lock, so
//! two workers can never be given the same byte.

use std::sync::{Mutex, MutexGuard};

/// A byte range `[start, end)` given to one worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim {
    /// Which segment this is. Pass it back to the scheduler.
    pub id: usize,
    /// First byte of the segment.
    pub start: u64,
    /// One past the last byte, as it was when the claim was made. The scheduler
    /// may shrink the segment later; [`Scheduler::reserve`] says when.
    pub end: u64,
}

#[derive(Debug)]
struct Segment {
    start: u64,
    /// The next byte that has not been handed out for writing.
    pos: u64,
    end: u64,
}

/// Splits `total` bytes into at most `parts` ranges of nearly equal size.
///
/// No range is smaller than `min_segment` (unless the whole file is), so a
/// small file is not split into many tiny requests. Returns `(start, end)`
/// pairs that touch each other and together cover `0..total`.
pub fn split_evenly(total: u64, parts: usize, min_segment: u64) -> Vec<(u64, u64)> {
    if total == 0 {
        return Vec::new();
    }
    let by_size = (total / min_segment.max(1)).max(1);
    let parts = (parts.max(1) as u64).min(by_size);
    let base = total / parts;
    let extra = total % parts;
    let mut ranges = Vec::with_capacity(parts as usize);
    let mut start = 0;
    for index in 0..parts {
        let len = base + u64::from(index < extra);
        ranges.push((start, start + len));
        start += len;
    }
    ranges
}

/// Hands out and re-divides the work of one download.
#[derive(Debug)]
pub struct Scheduler {
    min_split: u64,
    segments: Mutex<Vec<Segment>>,
}

impl Scheduler {
    /// Plan a download of `total` bytes over up to `connections` connections.
    ///
    /// `min_segment` is the smallest piece worth its own connection. Both the
    /// first split and later re-splitting respect it.
    pub fn new(total: u64, connections: usize, min_segment: u64) -> Self {
        let segments = split_evenly(total, connections, min_segment)
            .into_iter()
            .map(|(start, end)| Segment {
                start,
                pos: start,
                end,
            })
            .collect();
        Self {
            min_split: min_segment.max(1),
            segments: Mutex::new(segments),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Segment>> {
        // A worker that panicked cannot leave the list half-edited: every
        // change below is a couple of plain assignments.
        self.segments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The segments that exist right now, as claims. Call it once, right
    /// after [`Scheduler::new`], to start the first workers.
    pub fn initial_claims(&self) -> Vec<Claim> {
        self.lock()
            .iter()
            .enumerate()
            .map(|(id, segment)| Claim {
                id,
                start: segment.start,
                end: segment.end,
            })
            .collect()
    }

    /// Ask to write up to `want` bytes for segment `id`.
    ///
    /// Returns how many bytes the worker may write, at the segment's current
    /// position. `0` means the segment is finished (or was shrunk and the
    /// worker has reached its new end), so the worker must stop and drop
    /// whatever else it received.
    pub fn reserve(&self, id: usize, want: u64) -> u64 {
        let mut segments = self.lock();
        let segment = &mut segments[id];
        let granted = want.min(segment.end - segment.pos);
        segment.pos += granted;
        granted
    }

    /// `(start, position, end)` of segment `id`: where it began, how far it
    /// has been handed out, and where it ends now.
    pub fn state(&self, id: usize) -> (u64, u64, u64) {
        let segments = self.lock();
        let segment = &segments[id];
        (segment.start, segment.pos, segment.end)
    }

    /// Take the second half of the biggest unfinished segment, to give to a
    /// worker that has run out of work.
    ///
    /// Returns `None` when every segment is too small to be worth splitting.
    /// The segment that was cut keeps its first half; its worker notices
    /// through [`Scheduler::reserve`].
    pub fn steal(&self) -> Option<Claim> {
        let mut segments = self.lock();
        let (victim, remaining) = segments
            .iter()
            .enumerate()
            .map(|(id, segment)| (id, segment.end - segment.pos))
            .max_by_key(|&(_, remaining)| remaining)?;
        if remaining < self.min_split.saturating_mul(2) {
            return None;
        }
        let old_end = segments[victim].end;
        let new_start = old_end - remaining / 2;
        segments[victim].end = new_start;
        segments.push(Segment {
            start: new_start,
            pos: new_start,
            end: old_end,
        });
        Some(Claim {
            id: segments.len() - 1,
            start: new_start,
            end: old_end,
        })
    }

    /// Bytes not yet handed out for writing, over all segments.
    pub fn remaining(&self) -> u64 {
        self.lock()
            .iter()
            .map(|segment| segment.end - segment.pos)
            .sum()
    }

    /// How many segments exist (including ones created by re-splitting).
    pub fn segment_count(&self) -> usize {
        self.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::{split_evenly, Scheduler};

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn even_split_covers_everything_without_gaps() {
        for total in [1, 7, 100, 1_000_003, 10 * MIB + 5] {
            for parts in [1, 2, 3, 8, 64] {
                let ranges = split_evenly(total, parts, 1);
                assert_eq!(ranges.first().unwrap().0, 0);
                assert_eq!(ranges.last().unwrap().1, total);
                for pair in ranges.windows(2) {
                    assert_eq!(pair[0].1, pair[1].0, "gap or overlap");
                }
                let sizes: Vec<u64> = ranges.iter().map(|(a, b)| b - a).collect();
                let (min, max) = (sizes.iter().min().unwrap(), sizes.iter().max().unwrap());
                assert!(max - min <= 1, "sizes {sizes:?} are not even");
            }
        }
    }

    #[test]
    fn small_files_are_not_cut_into_tiny_pieces() {
        // 3 MiB with a 1 MiB minimum can have at most 3 pieces.
        assert_eq!(split_evenly(3 * MIB, 8, MIB).len(), 3);
        // Smaller than the minimum: one piece, whatever was asked.
        assert_eq!(split_evenly(100, 8, MIB), vec![(0, 100)]);
        assert_eq!(split_evenly(10 * MIB, 0, MIB).len(), 1);
    }

    #[test]
    fn an_empty_file_has_no_segments() {
        assert!(split_evenly(0, 4, MIB).is_empty());
    }

    #[test]
    fn reserve_hands_out_bytes_up_to_the_end() {
        let scheduler = Scheduler::new(100, 1, 1);
        assert_eq!(scheduler.reserve(0, 60), 60);
        assert_eq!(scheduler.reserve(0, 60), 40);
        assert_eq!(scheduler.reserve(0, 60), 0);
        assert_eq!(scheduler.remaining(), 0);
    }

    #[test]
    fn steal_takes_the_second_half_of_the_biggest_remainder() {
        let scheduler = Scheduler::new(10 * MIB, 2, MIB);
        // Segment 0 is nearly done; segment 1 has all of its 5 MiB left.
        assert_eq!(scheduler.reserve(0, 5 * MIB - 10), 5 * MIB - 10);
        let claim = scheduler.steal().unwrap();
        assert_eq!(claim.id, 2);
        let new_start = 5 * MIB + (5 * MIB) / 2;
        assert_eq!((claim.start, claim.end), (new_start, 10 * MIB));
        // The cut segment now ends where the new one begins.
        assert_eq!(scheduler.state(1), (5 * MIB, 5 * MIB, new_start));
        assert_eq!(scheduler.state(2), (new_start, new_start, 10 * MIB));
    }

    #[test]
    fn a_cut_worker_is_told_to_stop_at_its_new_end() {
        let scheduler = Scheduler::new(4 * MIB, 1, MIB);
        assert_eq!(scheduler.reserve(0, MIB), MIB);
        let claim = scheduler.steal().unwrap();
        assert_eq!(claim.start, MIB + (3 * MIB) / 2);
        // The old worker may still write up to the new end, not beyond.
        let allowed = claim.start - MIB;
        assert_eq!(scheduler.reserve(0, 10 * MIB), allowed);
        assert_eq!(scheduler.reserve(0, 1), 0);
    }

    #[test]
    fn nothing_is_stolen_when_every_remainder_is_small() {
        let scheduler = Scheduler::new(4 * MIB, 2, MIB);
        // 2 MiB each; reserve until each has less than 2 MiB left.
        scheduler.reserve(0, 1);
        scheduler.reserve(1, 1);
        assert!(scheduler.steal().is_none());
    }

    /// Whatever happens, the segments must tile the file exactly, and every
    /// byte is handed out once.
    #[test]
    fn random_use_never_loses_or_duplicates_bytes() {
        let total = 50 * MIB + 12_345;
        let scheduler = Scheduler::new(total, 4, 256 * 1024);
        let mut handed_out = 0u64;
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..5_000 {
            let count = scheduler.segment_count();
            if next() % 7 == 0 {
                scheduler.steal();
            } else {
                let id = (next() % count as u64) as usize;
                let want = next() % (2 * MIB);
                handed_out += scheduler.reserve(id, want);
            }
        }
        // Drain whatever is left.
        for id in 0..scheduler.segment_count() {
            handed_out += scheduler.reserve(id, u64::MAX);
        }
        assert_eq!(handed_out, total);
        assert_eq!(scheduler.remaining(), 0);

        let mut ranges: Vec<(u64, u64)> = (0..scheduler.segment_count())
            .map(|id| {
                let (start, _, end) = scheduler.state(id);
                (start, end)
            })
            .collect();
        ranges.sort();
        assert_eq!(ranges.first().unwrap().0, 0);
        assert_eq!(ranges.last().unwrap().1, total);
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "gap or overlap");
        }
    }
}
