//! Queue schedules: the days and times a queue may run, and the speed limit
//! that applies in each window.
//!
//! A schedule is a list of windows. The queue runs while any window is open
//! and is stopped (keeping its progress) while none is. Windows may cross
//! midnight (`22:00` to `06:00`). A queue without a schedule always runs.

use serde::{Deserialize, Serialize};

/// A point in the week, to the minute. Weekday 0 is Monday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Moment {
    /// 0 (Monday) to 6 (Sunday).
    pub weekday: u8,
    /// Minutes since midnight, 0 to 1439.
    pub minute: u16,
}

impl Moment {
    /// A moment from a weekday (0 = Monday), hour and minute.
    pub fn new(weekday: u8, hour: u16, minute: u16) -> Moment {
        Moment {
            weekday: weekday % 7,
            minute: (hour * 60 + minute) % 1440,
        }
    }

    /// The local time now.
    pub fn now() -> Moment {
        use chrono::{Datelike, Timelike};
        let now = chrono::Local::now();
        Moment {
            weekday: now.weekday().num_days_from_monday() as u8,
            minute: (now.hour() * 60 + now.minute()) as u16,
        }
    }

    fn plus_minutes(self, minutes: u32) -> Moment {
        let total = self.minute as u32 + minutes;
        Moment {
            weekday: ((self.weekday as u32 + total / 1440) % 7) as u8,
            minute: (total % 1440) as u16,
        }
    }
}

const DAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// What is stored (and shown) for one window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    /// Days the window opens on (`mon` ... `sun`); empty means every day.
    /// For a window that crosses midnight this is the day it opens.
    #[serde(default)]
    pub days: Vec<String>,
    /// Opening time, `HH:MM`.
    pub start: String,
    /// Closing time, `HH:MM`. Earlier than `start` means after midnight.
    pub end: String,
    /// Speed limit in bytes per second while this window is open. It
    /// replaces the queue's own limit.
    #[serde(default)]
    pub speed_limit: Option<u64>,
}

/// A queue's schedule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    /// When the queue may run.
    pub windows: Vec<Window>,
}

fn parse_time(text: &str) -> Result<u16, String> {
    let (hour, minute) = text
        .trim()
        .split_once(':')
        .ok_or_else(|| format!("'{text}' is not a time like 07:30"))?;
    let (hour, minute): (u16, u16) = (
        hour.parse()
            .map_err(|_| format!("'{text}' is not a time like 07:30"))?,
        minute
            .parse()
            .map_err(|_| format!("'{text}' is not a time like 07:30"))?,
    );
    if hour > 23 || minute > 59 {
        return Err(format!("'{text}' is not a time of day"));
    }
    Ok(hour * 60 + minute)
}

/// Read days such as `mon-fri`, `sat,sun`, `mon,wed-fri` or `all`.
pub fn parse_days(text: &str) -> Result<Vec<String>, String> {
    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() || text == "all" || text == "daily" {
        return Ok(Vec::new());
    }
    let index = |name: &str| {
        DAY_NAMES
            .iter()
            .position(|d| *d == name.trim())
            .ok_or_else(|| format!("'{name}' is not a day; use mon, tue, wed, thu, fri, sat, sun"))
    };
    let mut days = Vec::new();
    for part in text.split(',') {
        match part.split_once('-') {
            Some((from, to)) => {
                let (from, to) = (index(from)?, index(to)?);
                let mut day = from;
                loop {
                    days.push(DAY_NAMES[day].to_string());
                    if day == to {
                        break;
                    }
                    day = (day + 1) % 7;
                }
            }
            None => days.push(DAY_NAMES[index(part)?].to_string()),
        }
    }
    days.sort_by_key(|d| DAY_NAMES.iter().position(|n| n == d));
    days.dedup();
    Ok(days)
}

impl Window {
    fn checked(&self) -> Result<(u16, u16, [bool; 7]), String> {
        let (start, end) = (parse_time(&self.start)?, parse_time(&self.end)?);
        if start == end {
            return Err("a window must not start and end at the same time".into());
        }
        let mut days = [self.days.is_empty(); 7];
        for name in &self.days {
            let index = DAY_NAMES
                .iter()
                .position(|d| *d == name.to_ascii_lowercase())
                .ok_or_else(|| format!("'{name}' is not a day"))?;
            days[index] = true;
        }
        Ok((start, end, days))
    }

    fn contains(&self, at: Moment) -> bool {
        let Ok((start, end, days)) = self.checked() else {
            return false;
        };
        let today = days[at.weekday as usize];
        let yesterday = days[(at.weekday as usize + 6) % 7];
        if start < end {
            today && (start..end).contains(&at.minute)
        } else {
            // Crosses midnight: the evening part belongs to today's opening,
            // the morning part to yesterday's.
            (today && at.minute >= start) || (yesterday && at.minute < end)
        }
    }
}

impl Schedule {
    /// Read a schedule from its stored JSON and check it.
    pub fn from_json(text: &str) -> Result<Schedule, String> {
        let schedule: Schedule =
            serde_json::from_str(text).map_err(|e| format!("not a schedule: {e}"))?;
        schedule.check()?;
        Ok(schedule)
    }

    /// The JSON to store.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Whether every window makes sense, and there is at least one.
    pub fn check(&self) -> Result<(), String> {
        if self.windows.is_empty() {
            return Err("a schedule needs at least one window".into());
        }
        self.windows
            .iter()
            .try_for_each(|w| w.checked().map(|_| ()))
    }

    /// The window that is open at `at`, if any.
    pub fn open_window(&self, at: Moment) -> Option<&Window> {
        self.windows.iter().find(|w| w.contains(at))
    }

    /// Minutes until the schedule opens or closes next (`None` if it never
    /// changes, e.g. a window that covers every minute of every day).
    pub fn minutes_to_change(&self, at: Moment) -> Option<u32> {
        let now_open = self.open_window(at).is_some();
        (1..=7 * 1440u32)
            .find(|ahead| self.open_window(at.plus_minutes(*ahead)).is_some() != now_open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(days: &str, start: &str, end: &str, limit: Option<u64>) -> Window {
        Window {
            days: parse_days(days).unwrap(),
            start: start.into(),
            end: end.into(),
            speed_limit: limit,
        }
    }

    const MON: u8 = 0;
    const TUE: u8 = 1;
    const SAT: u8 = 5;
    const SUN: u8 = 6;

    #[test]
    fn day_lists_are_read() {
        assert_eq!(parse_days("all").unwrap(), Vec::<String>::new());
        assert_eq!(parse_days("mon-wed").unwrap(), ["mon", "tue", "wed"]);
        assert_eq!(parse_days("sat,sun").unwrap(), ["sat", "sun"]);
        assert_eq!(parse_days("fri-mon").unwrap(), ["mon", "fri", "sat", "sun"]);
        assert_eq!(parse_days("Mon, wed-thu").unwrap(), ["mon", "wed", "thu"]);
        assert!(parse_days("funday").is_err());
    }

    #[test]
    fn a_daytime_window_opens_and_closes() {
        let s = Schedule {
            windows: vec![window("mon-fri", "01:00", "07:00", None)],
        };
        assert!(s.open_window(Moment::new(MON, 1, 0)).is_some());
        assert!(s.open_window(Moment::new(MON, 6, 59)).is_some());
        assert!(s.open_window(Moment::new(MON, 7, 0)).is_none());
        assert!(s.open_window(Moment::new(MON, 0, 59)).is_none());
        assert!(s.open_window(Moment::new(SAT, 3, 0)).is_none());
    }

    #[test]
    fn a_window_may_cross_midnight() {
        let s = Schedule {
            windows: vec![window("mon", "22:00", "06:00", None)],
        };
        assert!(s.open_window(Moment::new(MON, 23, 0)).is_some());
        // The morning part belongs to Monday's window: it is open on Tuesday morning.
        assert!(s.open_window(Moment::new(TUE, 5, 59)).is_some());
        assert!(s.open_window(Moment::new(TUE, 6, 0)).is_none());
        assert!(s.open_window(Moment::new(TUE, 23, 0)).is_none());
        assert!(s.open_window(Moment::new(MON, 5, 0)).is_none());
        // Sunday night into Monday works across the week's end too.
        let sun = Schedule {
            windows: vec![window("sun", "22:00", "06:00", None)],
        };
        assert!(sun.open_window(Moment::new(MON, 2, 0)).is_some());
        assert!(sun.open_window(Moment::new(SUN, 23, 0)).is_some());
    }

    #[test]
    fn every_day_when_no_days_are_given() {
        let s = Schedule {
            windows: vec![window("all", "12:00", "13:00", None)],
        };
        for day in 0..7 {
            assert!(s.open_window(Moment::new(day, 12, 30)).is_some());
        }
    }

    #[test]
    fn the_first_open_window_gives_the_speed_limit() {
        let s = Schedule {
            windows: vec![
                window("all", "09:00", "17:00", Some(500)),
                window("all", "00:00", "09:00", None),
            ],
        };
        assert_eq!(
            s.open_window(Moment::new(MON, 10, 0)).unwrap().speed_limit,
            Some(500)
        );
        assert_eq!(
            s.open_window(Moment::new(MON, 3, 0)).unwrap().speed_limit,
            None
        );
    }

    #[test]
    fn minutes_until_the_next_change() {
        let s = Schedule {
            windows: vec![window("mon-fri", "01:00", "07:00", None)],
        };
        assert_eq!(s.minutes_to_change(Moment::new(MON, 0, 30)), Some(30));
        assert_eq!(s.minutes_to_change(Moment::new(MON, 6, 0)), Some(60));
        // Friday 08:00: the next opening is Monday 01:00.
        assert_eq!(
            s.minutes_to_change(Moment::new(4, 8, 0)),
            Some((2 * 24 + 17) * 60)
        );
    }

    #[test]
    fn bad_schedules_are_refused() {
        assert!(Schedule { windows: vec![] }.check().is_err());
        assert!(Schedule {
            windows: vec![window("all", "08:00", "08:00", None)]
        }
        .check()
        .is_err());
        assert!(Schedule {
            windows: vec![window("all", "25:00", "08:00", None)]
        }
        .check()
        .is_err());
        assert!(Schedule::from_json("{}").is_err());
    }

    #[test]
    fn json_round_trips() {
        let s = Schedule {
            windows: vec![window("sat,sun", "00:00", "23:59", Some(1_000_000))],
        };
        assert_eq!(Schedule::from_json(&s.to_json()).unwrap(), s);
    }
}
