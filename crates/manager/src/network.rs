//! Noticing that the network changed under the downloads.
//!
//! When the machine moves from one Wi-Fi to another, or a VPN comes up or
//! goes down, connections that were open keep waiting for a network that is
//! gone. The manager watches the address the default route uses; when it
//! changes the running downloads are stopped (their progress is kept), and
//! when the new network has been stable for a moment they start again.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// Which local addresses the default routes use now. `None` means there is
/// no route for that address family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetworkState {
    /// The local IPv4 address used to reach the internet.
    pub v4: Option<IpAddr>,
    /// The local IPv6 address used to reach the internet.
    pub v6: Option<IpAddr>,
}

impl NetworkState {
    /// Whether there is any way out.
    pub fn online(&self) -> bool {
        self.v4.is_some() || self.v6.is_some()
    }
}

/// Reports the network state. Tests replace it.
pub trait NetworkProbe: Send + Sync {
    /// The state right now. Must not block for long.
    fn state(&self) -> NetworkState;
}

/// Asks the operating system which local address it would use for a
/// public address. A UDP "connection" sends nothing; it only makes the
/// system pick a route, so this is cheap and works without any rights.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemNetwork;

impl NetworkProbe for SystemNetwork {
    fn state(&self) -> NetworkState {
        let v4 = route_address(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53),
            "0.0.0.0:0",
        );
        let v6 = route_address(
            SocketAddr::new(
                IpAddr::V6(Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111)),
                53,
            ),
            "[::]:0",
        );
        NetworkState { v4, v6 }
    }
}

fn route_address(target: SocketAddr, bind: &str) -> Option<IpAddr> {
    let socket = UdpSocket::bind(bind).ok()?;
    socket.connect(target).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

/// What the watcher wants done after looking at the network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Nothing,
    /// The network changed: stop what is running and wait.
    Changed {
        online: bool,
    },
    /// The network has been steady and online: start again.
    Resume,
}

/// Decides when downloads must stop and when they may start again.
#[derive(Debug)]
pub(crate) struct Watch {
    last: NetworkState,
    holding: bool,
    steady_at: Instant,
}

impl Watch {
    /// Start watching. If there is no network yet, downloads are held until
    /// there is one.
    pub fn new(state: NetworkState, now: Instant) -> Self {
        Self {
            last: state,
            holding: !state.online(),
            steady_at: now,
        }
    }

    /// Whether downloads are being held back.
    pub fn holding(&self) -> bool {
        self.holding
    }

    /// Look at the network. `settle` is how long it must stay the same,
    /// and online, before downloads start again.
    pub fn observe(&mut self, state: NetworkState, now: Instant, settle: Duration) -> Step {
        if state != self.last {
            self.last = state;
            self.holding = true;
            self.steady_at = now + settle;
            return Step::Changed {
                online: state.online(),
            };
        }
        if self.holding && state.online() && now >= self.steady_at {
            self.holding = false;
            return Step::Resume;
        }
        Step::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(v4: Option<&str>) -> NetworkState {
        NetworkState {
            v4: v4.map(|a| a.parse().unwrap()),
            v6: None,
        }
    }

    const SETTLE: Duration = Duration::from_secs(3);

    #[test]
    fn nothing_happens_while_the_network_is_steady() {
        let t = Instant::now();
        let mut w = Watch::new(net(Some("192.168.1.5")), t);
        assert!(!w.holding());
        for s in 0..10 {
            let now = t + Duration::from_secs(s);
            assert_eq!(
                w.observe(net(Some("192.168.1.5")), now, SETTLE),
                Step::Nothing
            );
        }
    }

    #[test]
    fn a_new_address_stops_work_and_resumes_after_settling() {
        let t = Instant::now();
        let mut w = Watch::new(net(Some("192.168.1.5")), t);
        let step = w.observe(net(Some("10.0.0.7")), t, SETTLE);
        assert_eq!(step, Step::Changed { online: true });
        assert!(w.holding());
        let early = t + Duration::from_secs(2);
        assert_eq!(
            w.observe(net(Some("10.0.0.7")), early, SETTLE),
            Step::Nothing
        );
        let later = t + SETTLE;
        assert_eq!(
            w.observe(net(Some("10.0.0.7")), later, SETTLE),
            Step::Resume
        );
        assert!(!w.holding());
    }

    #[test]
    fn going_offline_waits_for_the_network_to_return() {
        let t = Instant::now();
        let mut w = Watch::new(net(Some("192.168.1.5")), t);
        assert_eq!(
            w.observe(net(None), t, SETTLE),
            Step::Changed { online: false }
        );
        // Hours later, still offline: still held.
        let much_later = t + Duration::from_secs(3600);
        assert_eq!(w.observe(net(None), much_later, SETTLE), Step::Nothing);
        assert!(w.holding());
        // Back: changed, then steady.
        let back = much_later + Duration::from_secs(1);
        assert_eq!(
            w.observe(net(Some("192.168.1.5")), back, SETTLE),
            Step::Changed { online: true }
        );
        assert_eq!(
            w.observe(net(Some("192.168.1.5")), back + SETTLE, SETTLE),
            Step::Resume
        );
    }

    #[test]
    fn flapping_keeps_postponing_the_restart() {
        let t = Instant::now();
        let mut w = Watch::new(net(Some("1.1.1.2")), t);
        w.observe(net(Some("1.1.1.3")), t, SETTLE);
        let step = w.observe(net(Some("1.1.1.4")), t + Duration::from_secs(2), SETTLE);
        assert_eq!(step, Step::Changed { online: true });
        // 3 s after the first change but only 1 s after the second.
        let now = t + Duration::from_secs(3);
        assert_eq!(w.observe(net(Some("1.1.1.4")), now, SETTLE), Step::Nothing);
        let now = t + Duration::from_secs(5);
        assert_eq!(w.observe(net(Some("1.1.1.4")), now, SETTLE), Step::Resume);
    }

    #[test]
    fn starting_offline_holds_until_online() {
        let t = Instant::now();
        let mut w = Watch::new(net(None), t);
        assert!(w.holding());
        assert_eq!(w.observe(net(None), t, SETTLE), Step::Nothing);
        assert_eq!(
            w.observe(net(Some("10.1.1.1")), t, SETTLE),
            Step::Changed { online: true }
        );
    }

    #[test]
    fn the_system_probe_does_not_panic() {
        // Whatever the machine's network is, asking must work.
        let _ = SystemNetwork.state().online();
    }
}
