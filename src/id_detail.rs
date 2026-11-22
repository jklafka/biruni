//! Detail country IDs for close zoom. Browser-free.
//!
//! The whole-world ID raster (`u_ids`, 4096 texels around on desktops, about
//! 10 km a texel) is too coarse to color countries up close: its edges sit
//! up to half a texel off the vector borders. So when the view is close
//! enough for that to show, the same IDs are rasterized again for a
//! lon/lat window around the view, at about a screen pixel per texel, and
//! `globe.frag` reads them there (`u_idsDetail`, `u_idWin`). Elsewhere the
//! world raster is used as before.
//!
//! The window is a lon/lat rectangle (equirectangular, like the world
//! raster, so the shader maps a texture `u`/`v` into it linearly). Its
//! texels are square on the ground at the latitude nearest the equator, so
//! the window never gets coarser than wanted anywhere in it.

use crate::camera::{spherical, Camera};

/// Detail starts when a world raster texel spans this many CSS pixels (the
/// fills used to fade out from 6).
pub const NEED_PX_PER_TEXEL: f64 = 3.0;
/// A detail texel spans about this many CSS pixels when the window is made.
const TARGET_PX_PER_TEXEL: f64 = 1.0;
/// The window is rebuilt when zooming has made its texels span more than
/// this many pixels (blocky), or fewer than this (wasted).
const MAX_PX_PER_TEXEL: f64 = 2.5;
const MIN_PX_PER_TEXEL: f64 = 0.35;
/// The window covers the view and this much more on each side (fraction of
/// the view's extent), so a pan does not leave it at once.
const PAD: f64 = 0.5;
/// Largest side and texel count of the window (R8: 4 MB).
pub const MAX_SIDE: usize = 2048;
const MAX_TEXELS: usize = 2048 * 2048;

/// A lon/lat window of detail IDs: `w x h` texels, row 0 at `lat_top`,
/// column 0 at `lon0`. Degrees; `lon0` is not wrapped and `dlon` is at most
/// 360.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub lon0: f64,
    pub lat_top: f64,
    pub dlon: f64,
    pub dlat: f64,
    pub w: usize,
    pub h: usize,
}

impl Window {
    /// `(u0, v0, du, dv)` in the world texture's coordinates
    /// (`u = lon/360 + 0.5`, `v = 0.5 - lat/180`), for `u_idWin`.
    pub fn uv(&self) -> [f32; 4] {
        [
            (self.lon0 / 360.0 + 0.5) as f32,
            (0.5 - self.lat_top / 180.0) as f32,
            (self.dlon / 360.0) as f32,
            (self.dlat / 180.0) as f32,
        ]
    }

    /// Latitude degrees per texel (the ground size of a texel).
    pub fn texel_deg(&self) -> f64 {
        self.dlat / self.h.max(1) as f64
    }

    /// The width of a world raster with this window's texel size, for
    /// `render::political_fill_amount`.
    pub fn equivalent_width(&self) -> usize {
        (360.0 / self.texel_deg()).round().max(1.0) as usize
    }

    /// Whether the point (degrees) is inside, at least `margin` texels from
    /// every edge.
    pub fn contains(&self, lat: f64, lon: f64, margin: f64) -> bool {
        let tw = self.dlon / self.w.max(1) as f64;
        let th = self.texel_deg();
        let dx = (lon - self.lon0).rem_euclid(360.0);
        let dy = self.lat_top - lat;
        dx >= margin * tw
            && dx <= self.dlon - margin * tw
            && dy >= margin * th
            && dy <= self.dlat - margin * th
    }
}

/// The view's extent: `(lat_lo, lat_hi, lon_lo, lon_hi)` in degrees, with
/// the longitudes relative to the view center's and unwrapped, and the
/// center's longitude. `None` if no part of the screen shows the map.
fn view_extent(cam: &Camera) -> Option<(f64, f64, f64, f64, f64)> {
    const N: usize = 8;
    let center_lon = cam.lon.to_degrees();
    let (mut lat_lo, mut lat_hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut lon_lo, mut lon_hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for j in 0..=N {
        for i in 0..=N {
            let (sx, sy) = (
                cam.width * i as f64 / N as f64,
                cam.height * j as f64 / N as f64,
            );
            let Some((lat, lon)) = cam.pick(sx, sy) else {
                continue;
            };
            let (lat, lon) = (lat.to_degrees(), lon.to_degrees());
            let rel = (lon - center_lon + 180.0).rem_euclid(360.0) - 180.0;
            lat_lo = lat_lo.min(lat);
            lat_hi = lat_hi.max(lat);
            lon_lo = lon_lo.min(rel);
            lon_hi = lon_hi.max(rel);
        }
    }
    if !(lat_lo.is_finite() && lon_lo.is_finite()) {
        return None;
    }
    // A pole on screen: every longitude is in view near it.
    for pole in [90.0f64, -90.0] {
        if let Some((x, y)) = cam.project(pole.to_radians(), 0.0) {
            if (0.0..=cam.width).contains(&x) && (0.0..=cam.height).contains(&y) {
                lon_lo = -180.0;
                lon_hi = 180.0;
                lat_lo = lat_lo.min(pole);
                lat_hi = lat_hi.max(pole);
            }
        }
    }
    Some((lat_lo, lat_hi, lon_lo, lon_hi, center_lon))
}

/// Radians of the sphere per CSS pixel at the screen center (`None` off the
/// map or if not finite).
pub fn angle_per_px(cam: &Camera) -> Option<f64> {
    const STEP: f64 = 4.0;
    let (cx, cy) = (cam.width / 2.0, cam.height / 2.0);
    let a = cam.pick(cx, cy - STEP / 2.0)?;
    let b = cam.pick(cx, cy + STEP / 2.0)?;
    let (va, vb) = (spherical(a.0, a.1), spherical(b.0, b.1));
    let d = (va[0] - vb[0]).hypot(va[1] - vb[1]).hypot(va[2] - vb[2]);
    let r = 2.0 * (d / 2.0).clamp(0.0, 1.0).asin() / STEP;
    (r.is_finite() && r > 0.0).then_some(r)
}

/// What to do about the detail window for this view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Plan {
    /// The world raster is fine here: no window needed.
    None,
    /// The current window still serves.
    Keep,
    /// Rasterize this window.
    Make(Window),
}

/// Decide, for a world raster `world_width` texels around and the window
/// now on the GPU (if any).
pub fn plan(cam: &Camera, world_width: usize, current: Option<&Window>) -> Plan {
    let Some(app) = angle_per_px(cam) else {
        return Plan::None;
    };
    let px_per_deg = 1.0 / app.to_degrees();
    let world_px = 360.0 / world_width.max(1) as f64 * px_per_deg;
    if world_px < NEED_PX_PER_TEXEL {
        return Plan::None;
    }
    let Some((lat_lo, lat_hi, lon_lo, lon_hi, center_lon)) = view_extent(cam) else {
        return Plan::None;
    };
    if let Some(c) = current {
        let px = c.texel_deg() * px_per_deg;
        let inside = |lat: f64, lon: f64| c.contains(lat, lon, 2.0);
        let covers = inside(lat_lo, center_lon + lon_lo)
            && inside(lat_lo, center_lon + lon_hi)
            && inside(lat_hi, center_lon + lon_lo)
            && inside(lat_hi, center_lon + lon_hi)
            && (lon_hi - lon_lo) < c.dlon;
        if covers && (MIN_PX_PER_TEXEL..=MAX_PX_PER_TEXEL).contains(&px) {
            return Plan::Keep;
        }
    }
    let (dlat_v, dlon_v) = (lat_hi - lat_lo, lon_hi - lon_lo);
    let lat_top = (lat_hi + PAD * dlat_v).min(90.0);
    let lat_bot = (lat_lo - PAD * dlat_v).max(-90.0);
    let (lon0, dlon) = if dlon_v * (1.0 + 2.0 * PAD) >= 360.0 {
        (center_lon - 180.0, 360.0)
    } else {
        (
            center_lon + lon_lo - PAD * dlon_v,
            dlon_v * (1.0 + 2.0 * PAD),
        )
    };
    let dlat = lat_top - lat_bot;
    if !(dlat > 0.0 && dlon > 0.0) {
        return Plan::None;
    }
    // Square texels at the latitude nearest the equator.
    let widest = if lat_bot <= 0.0 && lat_top >= 0.0 {
        1.0
    } else {
        lat_bot.abs().min(lat_top.abs()).to_radians().cos()
    };
    let mut texel = TARGET_PX_PER_TEXEL / px_per_deg;
    let size = |t: f64| {
        (
            ((dlon * widest / t).ceil() as usize).max(1),
            ((dlat / t).ceil() as usize).max(1),
        )
    };
    let (mut w, mut h) = size(texel);
    // Too big: coarser texels, keeping the window.
    let over = (w as f64 / MAX_SIDE as f64)
        .max(h as f64 / MAX_SIDE as f64)
        .max(((w * h) as f64 / MAX_TEXELS as f64).sqrt());
    if over > 1.0 {
        texel *= over;
        (w, h) = size(texel);
        w = w.min(MAX_SIDE);
        h = h.min(MAX_SIDE);
    }
    // Not worth it if the window is no finer than the world raster.
    if dlat / h as f64 >= 180.0 / (world_width / 2).max(1) as f64 {
        return Plan::None;
    }
    Plan::Make(Window {
        lon0,
        lat_top,
        dlon,
        dlat,
        w,
        h,
    })
}

/// Scanline filler for a window: each texel whose center is inside a
/// group of rings (even-odd, like the world raster) takes the group's ID.
/// Later groups win where they overlap.
pub struct Filler {
    pub win: Window,
    pub out: Vec<u8>,
    rows: Vec<Vec<f64>>,
}

impl Filler {
    pub fn new(win: Window) -> Filler {
        Filler {
            win,
            out: vec![0; win.w * win.h],
            rows: vec![Vec::new(); win.h],
        }
    }

    /// Fill one even-odd group of rings (lon/lat degree points, each ring
    /// within one copy of the world). `bbox` is the group's
    /// `[lon_min, lat_min, lon_max, lat_max]`.
    pub fn fill<'a>(
        &mut self,
        id: u8,
        bbox: [f64; 4],
        rings: impl Iterator<Item = &'a [[f64; 2]]> + Clone,
    ) {
        let win = self.win;
        let lat_bot = win.lat_top - win.dlat;
        if bbox[3] < lat_bot || bbox[1] > win.lat_top {
            return;
        }
        let th = win.texel_deg();
        let tw = win.dlon / win.w as f64;
        for shift in [-360.0, 0.0, 360.0] {
            if bbox[2] + shift < win.lon0 || bbox[0] + shift > win.lon0 + win.dlon {
                continue;
            }
            let mut any = false;
            for ring in rings.clone() {
                let n = ring.len();
                if n < 3 {
                    continue;
                }
                let mut j = n - 1;
                for i in 0..n {
                    let (a, b) = (ring[i], ring[j]);
                    j = i;
                    let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
                    if hi < lat_bot || lo > win.lat_top || lo == hi {
                        continue;
                    }
                    // Rows whose center latitude is in [lo, hi), widened by
                    // one and checked exactly below.
                    let r0 = (((win.lat_top - hi) / th - 0.5).floor().max(0.0)) as usize;
                    let r1 =
                        ((((win.lat_top - lo) / th - 0.5).ceil()).max(0.0) as usize).min(win.h - 1);
                    for r in r0..=r1 {
                        let lat = win.lat_top - (r as f64 + 0.5) * th;
                        if (a[1] > lat) != (b[1] > lat) {
                            let x = a[0] + (lat - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                            self.rows[r].push(x + shift);
                            any = true;
                        }
                    }
                }
            }
            if !any {
                continue;
            }
            for (r, xs) in self.rows.iter_mut().enumerate() {
                if xs.is_empty() {
                    continue;
                }
                xs.sort_by(|a, b| a.total_cmp(b));
                for pair in xs.as_chunks::<2>().0 {
                    // Texel c has its center at lon0 + (c + 0.5) tw.
                    let c0 = ((pair[0] - win.lon0) / tw - 0.5).ceil().max(0.0) as usize;
                    let c1 =
                        (((pair[1] - win.lon0) / tw - 0.5).ceil().max(0.0) as usize).min(win.w);
                    if c0 < c1 {
                        self.out[r * win.w + c0..r * win.w + c1].fill(id);
                    }
                }
                xs.clear();
            }
        }
    }
}

/// `[lon_min, lat_min, lon_max, lat_max]` of some rings.
pub fn bbox<'a>(rings: impl Iterator<Item = &'a [[f64; 2]]>) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for p in rings.flatten() {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::Projection;

    fn cam(lat: f64, lon: f64, alt_km: f64) -> Camera {
        let mut c = Camera::new(lat, lon, alt_km / crate::camera::EARTH_RADIUS_KM);
        c.width = 1200.0;
        c.height = 800.0;
        c
    }

    fn square(lon0: f64, lat0: f64, s: f64) -> Vec<[f64; 2]> {
        vec![
            [lon0, lat0],
            [lon0 + s, lat0],
            [lon0 + s, lat0 + s],
            [lon0, lat0 + s],
        ]
    }

    #[test]
    fn far_views_need_no_window() {
        assert_eq!(plan(&cam(47.0, 8.0, 12_000.0), 4096, None), Plan::None);
    }

    #[test]
    fn close_views_get_a_fine_window_around_the_view() {
        let c = cam(47.0, 8.0, 500.0);
        let Plan::Make(w) = plan(&c, 4096, None) else {
            panic!("expected a window");
        };
        assert!(w.contains(47.0, 8.0, 2.0));
        let px = w.texel_deg() / angle_per_px(&c).unwrap().to_degrees();
        assert!((0.5..=1.5).contains(&px), "{px} px per texel");
        assert!(w.w <= MAX_SIDE && w.h <= MAX_SIDE);
        assert!(w.equivalent_width() > 4096 * 4);
        // The same view keeps it; zooming in a lot, or panning away, does not.
        assert_eq!(plan(&c, 4096, Some(&w)), Plan::Keep);
        assert!(matches!(
            plan(&cam(47.0, 8.0, 100.0), 4096, Some(&w)),
            Plan::Make(_)
        ));
        assert!(matches!(
            plan(&cam(47.0, 30.0, 500.0), 4096, Some(&w)),
            Plan::Make(_)
        ));
    }

    #[test]
    fn windows_across_the_seam_and_over_a_pole() {
        let Plan::Make(w) = plan(&cam(-17.0, 179.5, 600.0), 4096, None) else {
            panic!("expected a window");
        };
        assert!(w.contains(-17.0, 179.9, 2.0) && w.contains(-17.0, -179.9, 2.0));
        let Plan::Make(p) = plan(&cam(89.0, 0.0, 700.0), 4096, None) else {
            panic!("expected a window");
        };
        assert_eq!(p.dlon, 360.0);
        assert!(p.lat_top >= 89.99);
    }

    #[test]
    fn flat_maps_get_windows_too() {
        let mut c = cam(10.0, 20.0, 300.0);
        c.set_projection(Projection::Mercator);
        assert!(matches!(plan(&c, 4096, None), Plan::Make(_)));
    }

    #[test]
    fn fills_texel_centers_inside_even_odd() {
        let win = Window {
            lon0: 0.0,
            lat_top: 10.0,
            dlon: 10.0,
            dlat: 10.0,
            w: 10,
            h: 10,
        };
        let mut f = Filler::new(win);
        let outer = square(2.0, 2.0, 6.0);
        let hole = square(4.0, 4.0, 2.0);
        let rings = [outer.as_slice(), hole.as_slice()];
        f.fill(7, bbox(rings.iter().copied()), rings.iter().copied());
        let at = |lon: usize, lat: usize| f.out[(9 - lat) * 10 + lon];
        assert_eq!(at(2, 2), 7);
        assert_eq!(at(7, 7), 7);
        assert_eq!(at(1, 5), 0);
        assert_eq!(at(8, 5), 0);
        assert_eq!(at(4, 4), 0, "the hole");
        assert_eq!(at(5, 5), 0, "the hole");
        assert_eq!(f.out.iter().filter(|&&v| v == 7).count(), 36 - 4);
    }

    #[test]
    fn fills_across_the_seam_and_skips_bad_rings() {
        // A window from 170 E to 190 E (170 W).
        let win = Window {
            lon0: 170.0,
            lat_top: 10.0,
            dlon: 20.0,
            dlat: 10.0,
            w: 20,
            h: 10,
        };
        let mut f = Filler::new(win);
        let east = square(-179.0, 0.0, 4.0); // 179 W .. 175 W
        let rings = [east.as_slice()];
        f.fill(3, bbox(rings.iter().copied()), rings.iter().copied());
        // Columns 11..15 (181..185 E), row 7 (2.5 N).
        assert_eq!(f.out[7 * 20 + 11], 3);
        assert_eq!(f.out[7 * 20 + 14], 3);
        assert_eq!(f.out[7 * 20 + 10], 0);
        assert_eq!(f.out[7 * 20 + 15], 0);
        let degenerate = vec![[171.0, 1.0], [172.0, 2.0]];
        let nan = vec![[f64::NAN, 1.0], [172.0, f64::NAN], [173.0, 3.0]];
        let rings = [degenerate.as_slice(), nan.as_slice()];
        f.fill(9, [171.0, 1.0, 173.0, 3.0], rings.iter().copied());
        assert!(!f.out.contains(&9));
    }

    #[test]
    fn matches_the_world_raster_layout() {
        // A window that is exactly a 360-texel world raster's patch fills
        // the same texels the world raster's rule would.
        let win = Window {
            lon0: -180.0,
            lat_top: 90.0,
            dlon: 360.0,
            dlat: 180.0,
            w: 36,
            h: 18,
        };
        let mut f = Filler::new(win);
        let sq = square(-15.0, -5.0, 30.0);
        let rings = [sq.as_slice()];
        f.fill(1, bbox(rings.iter().copied()), rings.iter().copied());
        // Columns with centers in [-15, 15): -15, -5, 5 (16, 17, 18), and
        // rows with centers in [-5, 25): 15, 5, -5 (7, 8, 9).
        let count = f.out.iter().filter(|&&v| v == 1).count();
        assert_eq!(count, 3 * 3);
        assert_eq!(f.out[7 * 36 + 18], 1);
    }
}
