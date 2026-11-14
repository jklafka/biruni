//! Dynamic resolution: how much to scale the render target while the view
//! is moving. Pure logic with no browser calls so it can be unit tested.
//!
//! While the camera moves the globe is shaded at a fraction of the canvas
//! resolution and upscaled. Once motion has stopped for `SETTLE_MS` one frame
//! is drawn at full resolution. The scale adapts between `LOW` and `HIGH`
//! to the measured frame interval.

/// Scale used when frames are slow.
pub const LOW: f64 = 0.5;
/// Scale used at first and when frames are fast.
pub const HIGH: f64 = 0.75;
/// Motion must have been absent this long before the full-resolution frame,
/// so a flick or a series of wheel notches does not flicker between sizes.
pub const SETTLE_MS: f64 = 120.0;
/// Canvases smaller than this many pixels are always drawn at full
/// resolution: the saving is small and the softness would be noticeable.
pub const MIN_PIXELS: f64 = 1_000_000.0;
/// Smoothed frame interval above which the scale drops to `LOW`.
const SLOW_MS: f64 = 22.0;
/// Smoothed frame interval below which the scale returns to `HIGH`.
const FAST_MS: f64 = 11.0;
/// Minimum time between two scale changes while moving.
const COOLDOWN_MS: f64 = 400.0;

/// The `dynres` query parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Setting {
    /// Adapt to frame time, and only on large canvases.
    Auto,
    /// Never scale (`?dynres=off`).
    Off,
    /// Always use this scale while moving (`?dynres=0.5`), at any size.
    Fixed(f64),
}

/// Parse the value of the `dynres` query parameter.
pub fn parse_setting(search: &str) -> Setting {
    let value = search
        .trim_start_matches('?')
        .split('&')
        .find_map(|kv| kv.strip_prefix("dynres="));
    match value {
        Some("off") => Setting::Off,
        Some(v) => match v.parse::<f64>() {
            Ok(s) if s.is_finite() && (0.25..1.0).contains(&s) => Setting::Fixed(s),
            Ok(s) if s.is_finite() && s >= 1.0 => Setting::Off,
            _ => Setting::Auto,
        },
        None => Setting::Auto,
    }
}

#[derive(Debug)]
pub struct Quality {
    setting: Setting,
    /// Scale to use while moving.
    moving_scale: f64,
    was_moving: bool,
    last_motion_ms: f64,
    last_change_ms: f64,
    /// Smoothed frame interval while moving.
    ema_ms: f64,
}

impl Quality {
    pub fn new(setting: Setting) -> Self {
        Self {
            setting,
            moving_scale: match setting {
                Setting::Fixed(s) => s,
                _ => HIGH,
            },
            was_moving: false,
            last_motion_ms: f64::NEG_INFINITY,
            last_change_ms: f64::NEG_INFINITY,
            ema_ms: 16.7,
        }
    }

    /// Render scale for this frame (1.0 = full resolution).
    ///
    /// `moving` comes from `Controls::in_motion`; `dt_ms` is the time since
    /// the previous frame and `pixels` the canvas area in device pixels.
    pub fn scale(&mut self, moving: bool, now_ms: f64, dt_ms: f64, pixels: f64) -> f64 {
        if !now_ms.is_finite() {
            return 1.0;
        }
        match self.setting {
            Setting::Off => return 1.0,
            Setting::Auto if pixels < MIN_PIXELS => return 1.0,
            _ => {}
        }
        if moving {
            if !self.was_moving {
                self.ema_ms = 16.7;
            }
            self.last_motion_ms = now_ms;
            if dt_ms.is_finite() && dt_ms > 0.0 {
                self.ema_ms += 0.2 * (dt_ms - self.ema_ms);
            }
            if self.setting == Setting::Auto && now_ms - self.last_change_ms >= COOLDOWN_MS {
                let next = if self.ema_ms > SLOW_MS {
                    LOW
                } else if self.ema_ms < FAST_MS {
                    HIGH
                } else {
                    self.moving_scale
                };
                if next != self.moving_scale {
                    self.moving_scale = next;
                    self.last_change_ms = now_ms;
                }
            }
        }
        self.was_moving = moving;
        if moving || now_ms - self.last_motion_ms < SETTLE_MS {
            self.moving_scale
        } else {
            1.0
        }
    }
}

/// Size of the scaled render target for a canvas of `w × h` at `scale`.
/// Never smaller than 1 pixel; exactly the canvas size at scale 1.
pub fn scaled_size(w: u32, h: u32, scale: f64) -> (u32, u32) {
    if scale >= 1.0 || !scale.is_finite() || scale <= 0.0 {
        return (w, h);
    }
    let f = |v: u32| ((v as f64 * scale).round() as u32).max(1);
    (f(w), f(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIG: f64 = 8_000_000.0;

    #[test]
    fn idle_is_full_resolution() {
        let mut q = Quality::new(Setting::Auto);
        assert_eq!(q.scale(false, 1000.0, 16.7, BIG), 1.0);
    }

    #[test]
    fn moving_scales_down_then_settles_after_the_delay() {
        let mut q = Quality::new(Setting::Auto);
        assert_eq!(q.scale(true, 1000.0, 16.7, BIG), HIGH);
        assert_eq!(q.scale(true, 1016.0, 16.7, BIG), HIGH);
        // Just stopped: still low until SETTLE_MS has passed.
        assert_eq!(q.scale(false, 1050.0, 16.7, BIG), HIGH);
        assert_eq!(q.scale(false, 1016.0 + SETTLE_MS - 1.0, 16.7, BIG), HIGH);
        assert_eq!(q.scale(false, 1016.0 + SETTLE_MS, 16.7, BIG), 1.0);
        assert_eq!(q.scale(false, 5000.0, 16.7, BIG), 1.0);
    }

    #[test]
    fn small_canvases_are_never_scaled() {
        let mut q = Quality::new(Setting::Auto);
        assert_eq!(q.scale(true, 0.0, 16.7, 800.0 * 600.0), 1.0);
    }

    #[test]
    fn slow_frames_drop_to_low_and_fast_frames_recover() {
        let mut q = Quality::new(Setting::Auto);
        let mut t = 0.0;
        let mut last = 0.0;
        for _ in 0..60 {
            t += 40.0;
            last = q.scale(true, t, 40.0, BIG);
        }
        assert_eq!(last, LOW);
        for _ in 0..200 {
            t += 8.0;
            last = q.scale(true, t, 8.0, BIG);
        }
        assert_eq!(last, HIGH);
    }

    #[test]
    fn scale_changes_are_rate_limited() {
        let mut q = Quality::new(Setting::Auto);
        let mut changes = 0;
        let mut prev = HIGH;
        let mut t = 0.0;
        for i in 0..400 {
            // Alternate slow and fast frames.
            let dt = if i % 40 < 20 { 50.0 } else { 5.0 };
            t += dt;
            let s = q.scale(true, t, dt, BIG);
            if s != prev {
                changes += 1;
                prev = s;
            }
        }
        assert!(changes as f64 <= t / COOLDOWN_MS + 1.0, "{changes}");
    }

    #[test]
    fn off_and_fixed_settings() {
        let mut off = Quality::new(Setting::Off);
        assert_eq!(off.scale(true, 0.0, 16.7, BIG), 1.0);
        let mut fixed = Quality::new(Setting::Fixed(0.5));
        // Applies at any size and ignores frame time.
        assert_eq!(fixed.scale(true, 0.0, 100.0, 1000.0), 0.5);
        assert_eq!(fixed.scale(true, 1000.0, 2.0, 1000.0), 0.5);
        assert_eq!(fixed.scale(false, 2000.0, 2.0, 1000.0), 1.0);
    }

    #[test]
    fn bad_numbers_never_panic_or_stick() {
        let mut q = Quality::new(Setting::Auto);
        assert_eq!(q.scale(true, f64::NAN, 16.0, BIG), 1.0);
        assert_eq!(q.scale(true, 10.0, f64::NAN, BIG), HIGH);
        assert_eq!(q.scale(true, 20.0, f64::INFINITY, BIG), HIGH);
        assert_eq!(q.scale(false, 1e6, 16.0, BIG), 1.0);
    }

    #[test]
    fn query_parsing() {
        assert_eq!(parse_setting(""), Setting::Auto);
        assert_eq!(parse_setting("?dynres=off"), Setting::Off);
        assert_eq!(parse_setting("?x=1&dynres=0.5"), Setting::Fixed(0.5));
        assert_eq!(parse_setting("?dynres=1"), Setting::Off);
        assert_eq!(parse_setting("?dynres=0"), Setting::Auto);
        assert_eq!(parse_setting("?dynres=nan"), Setting::Auto);
        assert_eq!(parse_setting("?dynres=abc"), Setting::Auto);
    }

    #[test]
    fn scaled_size_rounds_and_never_hits_zero() {
        assert_eq!(scaled_size(1920, 1080, 1.0), (1920, 1080));
        assert_eq!(scaled_size(1920, 1080, 0.5), (960, 540));
        assert_eq!(scaled_size(3840, 2160, 0.75), (2880, 1620));
        assert_eq!(scaled_size(1, 1, 0.5), (1, 1));
        assert_eq!(scaled_size(100, 100, f64::NAN), (100, 100));
    }
}
