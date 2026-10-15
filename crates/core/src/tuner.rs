//! Finding out how many connections a server is worth.
//!
//! More connections usually mean more speed, but not always: some servers
//! give each client a fixed share of bandwidth, and some punish clients that
//! open many connections, so the total gets *slower*. The tuner decides by
//! measuring. It watches the total speed with the connections running now,
//! adds as many again, waits for the speed to settle, and keeps the extra
//! connections only if the total speed really went up. Otherwise it takes the
//! extra connections back and stops trying.
//!
//! The tuner only decides; the caller starts and stops the connections.

/// Speed samples ignored at the start (the first bytes are slow while the
/// connection warms up).
const WARMUP: u32 = 2;
/// Samples averaged for one speed reading.
const WINDOW: usize = 4;
/// Samples to wait after adding connections before measuring again.
const SETTLE: u32 = 2;
/// How much faster the total must get to keep the added connections.
const GAIN: f64 = 1.2;

/// What the tuner wants done.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Nothing yet.
    None,
    /// Start up to this many more connections, then call [`Tuner::grew`].
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
    },
    Settling {
        before: f64,
        added: usize,
        waited: u32,
    },
    Judging {
        before: f64,
        added: usize,
    },
    Done,
}

#[derive(Debug)]
pub(crate) struct Tuner {
    phase: Phase,
    samples: Vec<f64>,
}

impl Tuner {
    pub(crate) fn new() -> Self {
        Self {
            phase: Phase::Warmup(0),
            samples: Vec::new(),
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
                self.phase = Phase::Pending { before };
                Action::Grow(running)
            }
            Phase::Settling {
                before,
                added,
                waited,
            } => {
                self.phase = if waited + 1 >= SETTLE {
                    self.samples.clear();
                    Phase::Judging { before, added }
                } else {
                    Phase::Settling {
                        before,
                        added,
                        waited: waited + 1,
                    }
                };
                Action::None
            }
            Phase::Judging { before, added } => {
                self.samples.push(speed);
                if self.samples.len() < WINDOW {
                    return Action::None;
                }
                let after = self.take_mean();
                if after > before * GAIN {
                    self.phase = Phase::Pending { before: after };
                    Action::Grow(running)
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
        if let Phase::Pending { before } = self.phase {
            self.phase = if started == 0 {
                Phase::Done
            } else {
                self.samples.clear();
                Phase::Settling {
                    before,
                    added: started,
                    waited: 0,
                }
            };
        }
    }

    fn take_mean(&mut self) -> f64 {
        let mean = self.samples.iter().sum::<f64>() / self.samples.len() as f64;
        self.samples.clear();
        mean
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Tuner, SETTLE, WARMUP, WINDOW};

    /// Feed samples at `speed` until the tuner says something.
    fn run(tuner: &mut Tuner, speed: f64, running: usize) -> Action {
        for _ in 0..(WARMUP as usize + WINDOW + SETTLE as usize + 5) {
            match tuner.sample(speed, running) {
                Action::None => {}
                other => return other,
            }
        }
        Action::None
    }

    #[test]
    fn keeps_growing_while_the_speed_rises() {
        let mut tuner = Tuner::new();
        assert_eq!(run(&mut tuner, 1000.0, 1), Action::Grow(1));
        tuner.grew(1);
        assert_eq!(run(&mut tuner, 1900.0, 2), Action::Grow(2));
        tuner.grew(2);
        assert_eq!(run(&mut tuner, 3500.0, 4), Action::Grow(4));
        assert!(!tuner.is_done());
    }

    #[test]
    fn takes_the_extra_connections_back_when_speed_does_not_rise() {
        let mut tuner = Tuner::new();
        assert_eq!(run(&mut tuner, 1000.0, 1), Action::Grow(1));
        tuner.grew(1);
        // Two connections are no faster than one.
        assert_eq!(run(&mut tuner, 1050.0, 2), Action::Retire(1));
        assert!(tuner.is_done());
    }

    #[test]
    fn takes_them_back_when_speed_falls() {
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
}
