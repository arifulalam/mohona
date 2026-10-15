//! Finding out how many connections a server is worth.
//!
//! More connections usually mean more speed, but not always: some servers
//! give each client a fixed share of bandwidth, and some punish clients that
//! open many connections, so the total gets *slower*. The tuner decides by
//! measuring. It watches the total speed with the connections running now,
//! adds ONE more, waits for the speed to settle, and keeps it only if the
//! total speed really went up. Otherwise it takes the extra connection back
//! and stops trying.
//!
//! It is deliberately patient and gentle. A new TCP connection starts slow, so
//! the first measurement waits a couple of seconds; connections are added one
//! at a time, because a burst of new connections is what makes some servers
//! answer 429; and each step needs a clear gain, so noise in the speed does
//! not count as a gain.
//!
//! The tuner only decides; the caller starts and stops the connections.

/// Speed samples ignored at the start. A new connection is slow at first
/// (TCP slow start), and measuring then would make any extra connection look
/// like a big gain.
const WARMUP: u32 = 8;
/// Samples averaged for one speed reading.
const WINDOW: usize = 6;
/// Samples to wait after adding a connection before measuring again.
const SETTLE: u32 = 3;

/// How much faster the total must get to keep one more connection, when
/// `running` connections were open before. Going from 1 to 2 should nearly
/// double the speed, going from 7 to 8 adds only about 14%, so half of the
/// ideal gain is asked for, but never less than 8% (the noise level).
fn required_gain(running: usize) -> f64 {
    1.0 + (0.5 / running.max(1) as f64).max(0.08)
}

/// What the tuner wants done.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Nothing yet.
    None,
    /// Start this many more connections (up to), then call [`Tuner::grew`].
    Grow(usize),
    /// Stop this many connections: they did not help. The tuner is finished.
    Retire(usize),
}

#[derive(Debug)]
enum Phase {
    Warmup(u32),
    Baseline,
    /// Waiting for the caller to say how many connections it started.
    Pending {
        before: f64,
        running: usize,
    },
    Settling {
        before: f64,
        running: usize,
        added: usize,
        waited: u32,
    },
    Judging {
        before: f64,
        running: usize,
        added: usize,
    },
    Done,
}

#[derive(Debug)]
pub(crate) struct Tuner {
    phase: Phase,
    samples: Vec<f64>,
    last_mean: f64,
}

impl Tuner {
    pub(crate) fn new() -> Self {
        Self {
            phase: Phase::Warmup(0),
            samples: Vec::new(),
            last_mean: 0.0,
        }
    }

    /// Whether the tuner has nothing more to decide.
    pub(crate) fn is_done(&self) -> bool {
        matches!(self.phase, Phase::Done)
    }

    /// Stop tuning, for example because the server refused a connection.
    pub(crate) fn stop(&mut self) {
        self.phase = Phase::Done;
    }

    /// The average speed (bytes per second) behind the latest decision.
    pub(crate) fn last_mean(&self) -> f64 {
        self.last_mean
    }

    /// Feed one speed sample (bytes per second over the last interval, for
    /// the whole download) taken while `running` connections are active.
    pub(crate) fn sample(&mut self, speed: f64, running: usize) -> Action {
        match self.phase {
            Phase::Warmup(seen) => {
                self.phase = if seen + 1 >= WARMUP {
                    Phase::Baseline
                } else {
                    Phase::Warmup(seen + 1)
                };
                Action::None
            }
            Phase::Baseline => {
                self.samples.push(speed);
                if self.samples.len() < WINDOW {
                    return Action::None;
                }
                let before = self.take_mean();
                self.phase = Phase::Pending { before, running };
                Action::Grow(1)
            }
            Phase::Settling {
                before,
                running,
                added,
                waited,
            } => {
                self.phase = if waited + 1 >= SETTLE {
                    self.samples.clear();
                    Phase::Judging {
                        before,
                        running,
                        added,
                    }
                } else {
                    Phase::Settling {
                        before,
                        running,
                        added,
                        waited: waited + 1,
                    }
                };
                Action::None
            }
            Phase::Judging {
                before,
                running,
                added,
            } => {
                self.samples.push(speed);
                if self.samples.len() < WINDOW {
                    return Action::None;
                }
                let after = self.take_mean();
                if after > before * required_gain(running) {
                    self.phase = Phase::Pending {
                        before: after,
                        running: running + added,
                    };
                    Action::Grow(1)
                } else {
                    self.phase = Phase::Done;
                    Action::Retire(added)
                }
            }
            Phase::Pending { .. } | Phase::Done => Action::None,
        }
    }

    /// Tell the tuner how many connections were started after
    /// [`Action::Grow`]. Zero means there was nothing left to give them.
    pub(crate) fn grew(&mut self, started: usize) {
        if let Phase::Pending { before, running } = self.phase {
            self.phase = if started == 0 {
                Phase::Done
            } else {
                self.samples.clear();
                Phase::Settling {
                    before,
                    running,
                    added: started,
                    waited: 0,
                }
            };
        }
    }

    fn take_mean(&mut self) -> f64 {
        let mean = self.samples.iter().sum::<f64>() / self.samples.len() as f64;
        self.samples.clear();
        self.last_mean = mean;
        mean
    }
}

#[cfg(test)]
mod tests {
    use super::{required_gain, Action, Tuner, SETTLE, WARMUP, WINDOW};

    /// Feed samples at `speed` until the tuner says something.
    fn run(tuner: &mut Tuner, speed: f64, running: usize) -> Action {
        for _ in 0..(WARMUP as usize + 2 * WINDOW + SETTLE as usize + 5) {
            match tuner.sample(speed, running) {
                Action::None => {}
                other => return other,
            }
        }
        Action::None
    }

    #[test]
    fn asks_for_less_gain_the_more_connections_there_are() {
        assert!((required_gain(1) - 1.5).abs() < 1e-9);
        assert!((required_gain(2) - 1.25).abs() < 1e-9);
        assert!((required_gain(100) - 1.08).abs() < 1e-9);
    }

    #[test]
    fn adds_one_connection_at_a_time_while_the_speed_rises() {
        let mut tuner = Tuner::new();
        assert_eq!(run(&mut tuner, 1000.0, 1), Action::Grow(1));
        tuner.grew(1);
        assert_eq!(run(&mut tuner, 1900.0, 2), Action::Grow(1));
        tuner.grew(1);
        assert_eq!(run(&mut tuner, 2800.0, 3), Action::Grow(1));
        assert!(!tuner.is_done());
        assert!((tuner.last_mean() - 2800.0).abs() < 1e-6);
    }

    #[test]
    fn takes_the_extra_connection_back_when_speed_does_not_rise_enough() {
        let mut tuner = Tuner::new();
        assert_eq!(run(&mut tuner, 1000.0, 1), Action::Grow(1));
        tuner.grew(1);
        // Two connections are only 30% faster than one: not enough.
        assert_eq!(run(&mut tuner, 1300.0, 2), Action::Retire(1));
        assert!(tuner.is_done());
    }

    #[test]
    fn takes_it_back_when_speed_falls() {
        let mut tuner = Tuner::new();
        assert_eq!(run(&mut tuner, 1_400_000.0, 1), Action::Grow(1));
        tuner.grew(1);
        assert_eq!(run(&mut tuner, 170_000.0, 2), Action::Retire(1));
    }

    #[test]
    fn stops_when_there_is_nothing_left_to_start() {
        let mut tuner = Tuner::new();
        assert_eq!(run(&mut tuner, 1000.0, 1), Action::Grow(1));
        tuner.grew(0);
        assert!(tuner.is_done());
        assert_eq!(tuner.sample(5.0, 1), Action::None);
    }

    #[test]
    fn a_stalled_download_does_not_grow_forever() {
        let mut tuner = Tuner::new();
        assert_eq!(run(&mut tuner, 0.0, 1), Action::Grow(1));
        tuner.grew(1);
        assert_eq!(run(&mut tuner, 0.0, 2), Action::Retire(1));
    }

    #[test]
    fn a_slow_start_is_not_mistaken_for_a_gain() {
        // The first warm-up samples are slow; they must be ignored, so the
        // baseline is the real speed and a second connection that gives no
        // extra speed is not kept.
        let mut tuner = Tuner::new();
        for _ in 0..WARMUP {
            assert_eq!(tuner.sample(100.0, 1), Action::None);
        }
        assert_eq!(run(&mut tuner, 1000.0, 1), Action::Grow(1));
        tuner.grew(1);
        assert_eq!(run(&mut tuner, 1000.0, 2), Action::Retire(1));
    }
}
