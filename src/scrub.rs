//! Time scrubber math for the day/night terminator. Browser-free.
//!
//! Everything is UTC Unix seconds (no DST, no local zones: the local time
//! shown in the panel is a display offset supplied by the caller). The two
//! sliders edit one part of the time each: a minute of the UTC day (0 to
//! 1439) and a day of the year (1 to 365 or 366), so the other part stays
//! put. Play steps the time on its own.

use std::fmt::Write;

pub const DAY: f64 = 86_400.0;
pub const HOUR: f64 = 3_600.0;
/// Latest time the scrubber produces (2099-12-31 23:59:59 UTC). The sun
/// model is only accurate for 1950-2050, but stays smooth long after.
pub const MAX_T: f64 = 4_102_444_799.0;
/// Minutes in a UTC day; the time slider runs 0 to this minus one.
pub const MINUTES_PER_DAY: u32 = 1_440;

pub fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_year(year: i64) -> u32 {
    if is_leap(year) {
        366
    } else {
        365
    }
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date `(year, month 1-12, day 1-31)` for days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// A UTC instant split into calendar parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    /// Day of the year, 1 for January 1st.
    pub doy: u32,
    /// Seconds since 00:00 UTC, 0 to 86399.
    pub secs: u32,
}

/// Split a time into UTC parts. `None` for NaN, infinite or absurd times.
pub fn civil(t: f64) -> Option<Civil> {
    if !t.is_finite() || t.abs() > 1e12 {
        return None;
    }
    let days = (t / DAY).floor();
    let secs = (t - days * DAY).floor() as u32;
    let days = days as i64;
    let (year, month, day) = civil_from_days(days);
    let doy = (days - days_from_civil(year, 1, 1) + 1) as u32;
    Some(Civil {
        year,
        month,
        day,
        doy,
        secs: secs.min(86_399),
    })
}

/// Unix seconds of 00:00 UTC on day `doy` of `year`. `doy` is clamped to
/// the length of that year, so 366 in a common year means December 31st.
pub fn day_start(year: i64, doy: u32) -> f64 {
    let doy = doy.clamp(1, days_in_year(year));
    ((days_from_civil(year, 1, 1) + i64::from(doy) - 1) * 86_400) as f64
}

/// The time clamped into `[0, MAX_T]`; NaN becomes 0.
pub fn clamp_t(t: f64) -> f64 {
    if t.is_nan() {
        0.0
    } else {
        t.clamp(0.0, MAX_T)
    }
}

/// Slider value for the UTC time of day: whole minutes since midnight.
pub fn minute_of_day(t: f64) -> u32 {
    civil(t).map_or(0, |c| c.secs / 60)
}

/// Same date as `t`, at `minute` (clamped to 0..=1439) UTC.
pub fn with_minute_of_day(t: f64, minute: u32) -> f64 {
    let Some(c) = civil(t) else { return 0.0 };
    clamp_t(day_start(c.year, c.doy) + f64::from(minute.min(MINUTES_PER_DAY - 1)) * 60.0)
}

/// Day-of-year slider value for `t`.
pub fn day_of_year(t: f64) -> u32 {
    civil(t).map_or(1, |c| c.doy)
}

/// Largest day-of-year slider value for the year of `t`.
pub fn year_length_of(t: f64) -> u32 {
    civil(t).map_or(365, |c| days_in_year(c.year))
}

/// Same year and time of day as `t`, on day `doy` (clamped to the year).
pub fn with_day_of_year(t: f64, doy: u32) -> f64 {
    let Some(c) = civil(t) else { return 0.0 };
    clamp_t(day_start(c.year, doy) + f64::from(c.secs))
}

/// Move by `delta` seconds, staying within `[0, MAX_T]`.
pub fn shift(t: f64, delta: f64) -> f64 {
    clamp_t(t + delta)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `21 Jun 2024`, or an empty string for an unusable time.
pub fn format_date(t: f64) -> String {
    match civil(t) {
        Some(c) => format!("{} {} {}", c.day, MONTHS[c.month as usize - 1], c.year),
        None => String::new(),
    }
}

/// `HH:MM` for seconds since midnight (wrapped into one day).
pub fn format_clock(secs: i64) -> String {
    let m = secs.div_euclid(60).rem_euclid(1_440);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// Local time of day for the panel, e.g. `10:30` or `02:15 +1d` when the
/// local date differs from the UTC one. `offset_min` is minutes east of UTC
/// (the negative of JavaScript's `getTimezoneOffset`).
pub fn format_local(t: f64, offset_min: f64) -> String {
    let Some(c) = civil(t) else {
        return String::new();
    };
    let off = if offset_min.is_finite() {
        offset_min.round() as i64
    } else {
        0
    };
    let local = i64::from(c.secs) + off * 60;
    let mut s = format_clock(local);
    match local.div_euclid(86_400) {
        0 => {}
        d if d > 0 => {
            let _ = write!(s, " +{d}d");
        }
        d => {
            let _ = write!(s, " {d}d");
        }
    }
    s
}

/// What play advances.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayMode {
    /// The sun goes around the Earth: the date stays, the time runs on.
    #[default]
    Day,
    /// The seasons turn: the time of day stays, the date runs on and wraps
    /// from December back to January of the same year.
    Year,
}

impl PlayMode {
    pub fn toggled(self) -> PlayMode {
        match self {
            PlayMode::Day => PlayMode::Year,
            PlayMode::Year => PlayMode::Day,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PlayMode::Day => "Day",
            PlayMode::Year => "Year",
        }
    }
}

/// Real seconds for one simulated day in day mode.
pub const DAY_SECS_PER_DAY: f64 = 10.0;
/// Real seconds for one simulated year in year mode.
pub const YEAR_PLAY_SECS: f64 = 12.0;
/// With reduced motion, play jumps once per this many milliseconds, by one
/// hour (day mode) or one week (year mode). Nothing moves in between.
pub const REDUCED_STEP_MS: f64 = 700.0;

/// Play state: the mode and the leftover time between whole steps.
#[derive(Clone, Copy, Debug, Default)]
pub struct Player {
    pub mode: PlayMode,
    /// Reduced motion: coarse jumps instead of continuous motion.
    pub reduced: bool,
    /// Milliseconds (reduced) or fractional days (year mode) not yet applied.
    acc: f64,
    /// Year mode runs on into the next year instead of wrapping back to
    /// January (the solar system view, where the planets would jump).
    pub continuous: bool,
}

impl Player {
    pub fn new(mode: PlayMode, reduced: bool) -> Self {
        Self {
            mode,
            reduced,
            acc: 0.0,
            continuous: false,
        }
    }

    /// The time after `dt_ms` of play from `t`. Pass the frame time (the
    /// caller clamps it); the result is always inside `[0, MAX_T]`.
    pub fn advance(&mut self, t: f64, dt_ms: f64) -> f64 {
        let dt = if dt_ms.is_finite() {
            dt_ms.max(0.0)
        } else {
            0.0
        };
        if self.reduced {
            self.acc += dt;
            if self.acc < REDUCED_STEP_MS {
                return clamp_t(t);
            }
            self.acc %= REDUCED_STEP_MS;
            return match self.mode {
                PlayMode::Day => shift(t, HOUR),
                PlayMode::Year if self.continuous => shift(t, 7.0 * DAY),
                PlayMode::Year => year_days(t, 7),
            };
        }
        match self.mode {
            PlayMode::Day => shift(t, dt / 1000.0 / DAY_SECS_PER_DAY * DAY),
            PlayMode::Year => {
                let len = f64::from(year_length_of(t));
                self.acc += dt / 1000.0 / YEAR_PLAY_SECS * len;
                let whole = self.acc.floor();
                self.acc -= whole;
                if self.continuous {
                    shift(t, whole * DAY)
                } else {
                    year_days(t, whole as u32)
                }
            }
        }
    }
}

/// `t` moved `n` days later in its own year, wrapping December to January
/// and keeping the time of day.
pub fn year_days(t: f64, n: u32) -> f64 {
    let Some(c) = civil(t) else { return 0.0 };
    let len = days_in_year(c.year);
    let doy = (c.doy - 1 + n % len) % len + 1;
    with_day_of_year(t, doy)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T_2024_06_21_18: f64 = 1_718_992_800.0;

    fn unix(y: i64, m: i64, d: i64, hh: u32, mm: u32) -> f64 {
        (days_from_civil(y, m, d) * 86_400) as f64 + f64::from(hh * 3_600 + mm * 60)
    }

    #[test]
    fn civil_splits_known_instants() {
        let c = civil(T_2024_06_21_18).unwrap();
        assert_eq!((c.year, c.month, c.day), (2024, 6, 21));
        assert_eq!((c.doy, c.secs), (173, 18 * 3_600)); // leap year: Jun 21 is day 173
        let e = civil(0.0).unwrap();
        assert_eq!((e.year, e.month, e.day, e.doy, e.secs), (1970, 1, 1, 1, 0));
        let c = civil(951_782_400.0).unwrap(); // 2000-02-29
        assert_eq!((c.year, c.month, c.day, c.doy), (2000, 2, 29, 60));
    }

    #[test]
    fn civil_round_trips_every_day_of_several_years() {
        for year in [1999, 2000, 2023, 2024, 2096] {
            for doy in 1..=days_in_year(year) {
                let t = day_start(year, doy) + 12_345.0;
                let c = civil(t).unwrap();
                assert_eq!((c.year, c.doy, c.secs), (year, doy, 12_345));
            }
        }
    }

    #[test]
    fn leap_years() {
        assert!(is_leap(2024) && is_leap(2000));
        assert!(!is_leap(1900) && !is_leap(2100) && !is_leap(2023));
        assert_eq!(days_in_year(2024), 366);
        assert_eq!(days_in_year(2023), 365);
    }

    #[test]
    fn year_boundaries() {
        let nye = unix(2023, 12, 31, 23, 59);
        let c = civil(nye).unwrap();
        assert_eq!((c.year, c.doy), (2023, 365));
        let c = civil(nye + 60.0).unwrap();
        assert_eq!((c.year, c.month, c.day, c.doy, c.secs), (2024, 1, 1, 1, 0));
        let c = civil(unix(2024, 12, 31, 0, 0)).unwrap();
        assert_eq!(c.doy, 366);
        // Before 1970 still works.
        let c = civil(-1.0).unwrap();
        assert_eq!((c.year, c.month, c.day, c.secs), (1969, 12, 31, 86_399));
    }

    #[test]
    fn bad_times_are_rejected() {
        assert!(civil(f64::NAN).is_none());
        assert!(civil(f64::INFINITY).is_none());
        assert!(civil(1e13).is_none());
        assert_eq!(minute_of_day(f64::NAN), 0);
        assert_eq!(with_minute_of_day(f64::NAN, 5), 0.0);
        assert_eq!(with_day_of_year(f64::INFINITY, 5), 0.0);
        assert_eq!(clamp_t(f64::NAN), 0.0);
        assert_eq!(clamp_t(-5.0), 0.0);
        assert_eq!(clamp_t(1e15), MAX_T);
    }

    #[test]
    fn minute_slider_round_trips() {
        for m in [0, 1, 59, 60, 719, 720, 1_439] {
            let t = with_minute_of_day(T_2024_06_21_18, m);
            assert_eq!(minute_of_day(t), m);
            assert_eq!(day_of_year(t), 173, "the date does not move");
        }
        assert_eq!(with_minute_of_day(T_2024_06_21_18 + 1234.0, 0) % DAY, 0.0);
        // Out-of-range clamps to 23:59 rather than rolling to the next day.
        let late = with_minute_of_day(T_2024_06_21_18, 5_000);
        assert_eq!(minute_of_day(late), 1_439);
    }

    #[test]
    fn day_slider_keeps_time_of_day_and_year() {
        let t = T_2024_06_21_18 + 17.0; // 18:00:17
        for doy in [1, 60, 173, 366] {
            let u = with_day_of_year(t, doy);
            let c = civil(u).unwrap();
            assert_eq!((c.year, c.doy, c.secs), (2024, doy, 18 * 3_600 + 17));
        }
        // 366 in a common year clamps to December 31st.
        let c = civil(with_day_of_year(unix(2023, 6, 1, 6, 0), 366)).unwrap();
        assert_eq!((c.year, c.month, c.day), (2023, 12, 31));
        // 0 clamps to January 1st.
        assert_eq!(day_of_year(with_day_of_year(t, 0)), 1);
        assert_eq!(year_length_of(t), 366);
        assert_eq!(year_length_of(unix(2023, 1, 1, 0, 0)), 365);
    }

    #[test]
    fn solstices_and_equinox_by_day_of_year() {
        // 2025 (common year): Mar 20 = 79, Jun 21 = 172, Dec 21 = 355.
        let base = unix(2025, 1, 1, 12, 0);
        for (doy, month, day) in [(79, 3, 20), (172, 6, 21), (355, 12, 21)] {
            let c = civil(with_day_of_year(base, doy)).unwrap();
            assert_eq!((c.month, c.day), (month, day));
        }
    }

    #[test]
    fn shift_crosses_days_and_clamps() {
        let t = unix(2023, 12, 31, 23, 0);
        let c = civil(shift(t, HOUR)).unwrap();
        assert_eq!((c.year, c.doy, c.secs), (2024, 1, 0));
        assert_eq!(shift(t, -DAY), unix(2023, 12, 30, 23, 0));
        assert_eq!(shift(10.0, -DAY), 0.0);
        assert_eq!(shift(MAX_T, DAY), MAX_T);
    }

    #[test]
    fn formatting() {
        assert_eq!(format_date(T_2024_06_21_18), "21 Jun 2024");
        assert_eq!(format_date(f64::NAN), "");
        assert_eq!(format_clock(18 * 3_600), "18:00");
        assert_eq!(format_clock(-60), "23:59");
        assert_eq!(format_local(T_2024_06_21_18, -240.0), "14:00");
        assert_eq!(format_local(T_2024_06_21_18, 0.0), "18:00");
        assert_eq!(format_local(T_2024_06_21_18, 450.0), "01:30 +1d");
        assert_eq!(format_local(unix(2024, 6, 21, 1, 0), -300.0), "20:00 -1d");
        assert_eq!(format_local(T_2024_06_21_18, f64::NAN), "18:00");
    }

    #[test]
    fn day_play_runs_one_day_in_ten_seconds() {
        let mut p = Player::new(PlayMode::Day, false);
        let mut t = T_2024_06_21_18;
        for _ in 0..600 {
            t = p.advance(t, 1000.0 / 60.0); // ten seconds of 60 fps
        }
        let moved = t - T_2024_06_21_18;
        assert!((moved - DAY).abs() < 5.0, "{moved}");
    }

    #[test]
    fn year_play_keeps_time_of_day_and_wraps() {
        let mut p = Player::new(PlayMode::Year, false);
        let mut t = unix(2024, 1, 1, 6, 30);
        for _ in 0..(12 * 60) {
            t = p.advance(t, 1000.0 / 60.0);
            assert_eq!(civil(t).unwrap().secs, 6 * 3_600 + 30 * 60);
            assert_eq!(civil(t).unwrap().year, 2024);
        }
        // One full lap lands back near January 1st.
        let doy = day_of_year(t);
        assert!(doy <= 2 || doy >= 365, "{doy}");
        // A long frame cannot leave the year.
        let t2 = p.advance(unix(2024, 12, 30, 0, 0), 1e9);
        assert_eq!(civil(t2).unwrap().year, 2024);
    }

    #[test]
    fn continuous_year_play_runs_into_the_next_year() {
        let mut p = Player::new(PlayMode::Year, false);
        p.continuous = true;
        let mut t = unix(2024, 12, 20, 6, 30);
        for _ in 0..120 {
            t = p.advance(t, 1000.0 / 60.0);
        }
        // Two seconds of a 12-second year: about two months on, in 2025.
        let c = civil(t).unwrap();
        assert_eq!(c.year, 2025);
        assert_eq!(c.secs, 6 * 3_600 + 30 * 60);
        assert!((40..=55).contains(&c.doy), "{}", c.doy);
        // It stops at the end of the scrubber's range.
        assert_eq!(p.advance(MAX_T - 10.0, 1e9), MAX_T);
        let mut r = Player::new(PlayMode::Year, true);
        r.continuous = true;
        let d = r.advance(unix(2024, 12, 28, 0, 0), 800.0);
        assert_eq!(civil(d).unwrap().year, 2025);
    }

    #[test]
    fn reduced_motion_steps_coarsely() {
        let mut p = Player::new(PlayMode::Day, true);
        let t0 = T_2024_06_21_18;
        let mut t = t0;
        for _ in 0..40 {
            t = p.advance(t, 16.0); // 640 ms: no jump yet
        }
        assert_eq!(t, t0);
        for _ in 0..5 {
            t = p.advance(t, 16.0); // crosses 700 ms
        }
        assert_eq!(t, t0 + HOUR, "one hour step, nothing in between");
        let mut y = Player::new(PlayMode::Year, true);
        let d = y.advance(unix(2024, 12, 28, 0, 0), 800.0);
        assert_eq!(
            civil(d).unwrap().doy,
            4,
            "a week later, wrapped into January"
        );
        assert_eq!(civil(d).unwrap().year, 2024);
    }

    #[test]
    fn play_ignores_bad_dt() {
        let mut p = Player::new(PlayMode::Day, false);
        assert_eq!(p.advance(1000.0, f64::NAN), 1000.0);
        assert_eq!(p.advance(1000.0, -5.0), 1000.0);
        assert_eq!(p.advance(MAX_T, 100.0), MAX_T);
    }

    #[test]
    fn mode_toggles() {
        assert_eq!(PlayMode::Day.toggled(), PlayMode::Year);
        assert_eq!(PlayMode::Year.toggled().label(), "Day");
    }
}
