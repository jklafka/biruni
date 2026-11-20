//! Constellation figures and names for the sky view: decoding
//! `constellations.bin` (written by `scripts/convert_constellations.py` from
//! Stellarium's `western` sky culture and HYG positions, CC BY-SA; the format
//! is described there) and turning them into screen polylines and labels for
//! the overlay canvas. Browser-free.

use crate::sky::{mul, radec_vec, Mat3, SkyCamera};
use crate::stars::{overlaps, Rect};
use std::f64::consts::{FRAC_PI_2, TAU};

const MAGIC: &[u8; 4] = b"CNST";
const VERSION: u8 = 1;
const THIN: u8 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// A secondary line of the figure, drawn fainter.
    pub thin: bool,
    /// Equatorial unit vectors of the figure's stars.
    pub points: Vec<[f64; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Constellation {
    pub abbr: String,
    pub name: String,
    /// Where the name goes: equatorial unit vector.
    pub label: [f64; 3],
    pub lines: Vec<Line>,
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or("constellations.bin: truncated")?;
        let out = &self.data[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn point(&mut self) -> Result<[f64; 3], String> {
        let ra = self.u32()? as f64 / 4_294_967_296.0 * TAU;
        let dec = self.u32()? as i32 as f64 / i32::MAX as f64 * FRAC_PI_2;
        Ok(radec_vec(ra, dec))
    }
}

pub fn decode(data: &[u8]) -> Result<Vec<Constellation>, String> {
    let mut r = Reader { data, at: 0 };
    if r.take(4)? != MAGIC {
        return Err("constellations.bin: not a constellation file".into());
    }
    let version = r.u8()?;
    if version != VERSION {
        return Err(format!("constellations.bin: unsupported version {version}"));
    }
    r.take(3)?;
    let count = r.u32()?;
    let mut out = Vec::new();
    for _ in 0..count {
        let abbr = std::str::from_utf8(r.take(3)?)
            .map_err(|_| "constellations.bin: bad abbreviation")?
            .to_string();
        let len = r.u8()? as usize;
        let name = std::str::from_utf8(r.take(len)?)
            .map_err(|_| "constellations.bin: name is not UTF-8")?
            .to_string();
        let label = r.point()?;
        let n_lines = r.u16()?;
        let mut lines = Vec::with_capacity(n_lines as usize);
        for _ in 0..n_lines {
            let flags = r.u8()?;
            let n = r.u16()?;
            if n < 2 || flags & !THIN != 0 {
                return Err(format!("constellations.bin: {abbr}: bad polyline"));
            }
            let points = (0..n).map(|_| r.point()).collect::<Result<_, _>>()?;
            lines.push(Line {
                thin: flags & THIN != 0,
                points,
            });
        }
        out.push(Constellation {
            abbr,
            name,
            label,
            lines,
        });
    }
    if r.at != data.len() {
        return Err("constellations.bin: trailing bytes".into());
    }
    Ok(out)
}

/// Longest piece of a figure segment drawn as one straight screen line.
const STEP: f64 = 0.03; // radians, about 1.7 degrees
/// Pixels left clear at each end of a segment, so the line stops short of
/// its stars.
const GAP: f64 = 5.0;

fn slerp(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0);
    let w = dot.acos();
    if w < 1e-9 {
        return a;
    }
    let (sa, sb) = (((1.0 - t) * w).sin() / w.sin(), (t * w).sin() / w.sin());
    [0, 1, 2].map(|i| a[i] * sa + b[i] * sb)
}

/// Shorten a screen polyline by `gap` pixels at each end; `None` if too
/// little would be left.
fn trim(run: &[(f64, f64)], gap: f64) -> Option<Vec<(f64, f64)>> {
    let seg = |i: usize| (run[i + 1].0 - run[i].0).hypot(run[i + 1].1 - run[i].1);
    let total: f64 = (0..run.len() - 1).map(seg).sum();
    if total < 2.0 * gap + 4.0 {
        return None;
    }
    let point_at = |d: f64| {
        let mut left = d;
        for i in 0..run.len() - 1 {
            let l = seg(i);
            if left <= l || i == run.len() - 2 {
                let t = if l > 0.0 { (left / l).min(1.0) } else { 0.0 };
                return (
                    i,
                    (
                        run[i].0 + (run[i + 1].0 - run[i].0) * t,
                        run[i].1 + (run[i + 1].1 - run[i].1) * t,
                    ),
                );
            }
            left -= l;
        }
        (0, run[0])
    };
    let (i0, p0) = point_at(gap);
    let (i1, p1) = point_at(total - gap);
    let mut out = vec![p0];
    out.extend_from_slice(&run[i0 + 1..=i1]);
    out.push(p1);
    Some(out)
}

/// Screen polylines (CSS pixels) of the figures: each segment between two
/// stars as a sampled great-circle arc, split where it dips below the
/// horizon or leaves the neighborhood of the screen, and trimmed short of
/// the stars. The flag is `thin`.
pub fn screen_lines(
    figs: &[Constellation],
    cam: &SkyCamera,
    eq_to_hor: &Mat3,
) -> Vec<(bool, Vec<(f64, f64)>)> {
    let span = 4.0 * cam.width.max(cam.height);
    let near = |p: (f64, f64)| {
        p.0 > -span && p.0 < cam.width + span && p.1 > -span && p.1 < cam.height + span
    };
    let mut out = Vec::new();
    for c in figs {
        for line in &c.lines {
            for pair in line.points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let angle = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2])
                    .clamp(-1.0, 1.0)
                    .acos();
                let steps = ((angle / STEP).ceil() as usize).max(1);
                let mut run: Vec<(f64, f64)> = Vec::new();
                let mut runs = Vec::new();
                for i in 0..=steps {
                    let h = mul(eq_to_hor, slerp(a, b, i as f64 / steps as f64));
                    match cam.project(h).filter(|p| h[1] >= 0.0 && near(*p)) {
                        Some(p) => run.push(p),
                        None => {
                            if run.len() > 1 {
                                runs.push(std::mem::take(&mut run));
                            }
                            run.clear();
                        }
                    }
                }
                if run.len() > 1 {
                    runs.push(run);
                }
                // Only the ends that are stars get a gap: a run cut by the
                // horizon keeps its cut end, but trimming both is simpler and
                // the difference is a few pixels at the horizon line.
                for r in runs {
                    if let Some(t) = trim(&r, GAP) {
                        out.push((line.thin, t));
                    }
                }
            }
        }
    }
    out
}

/// Constellation names at their label points, for those above the horizon,
/// on screen and clear of `avoid` (the star names) and of each other:
/// (x, y, name) for text centered there, CSS pixels.
pub fn labels<'a>(
    figs: &'a [Constellation],
    cam: &SkyCamera,
    eq_to_hor: &Mat3,
    avoid: &[Rect],
) -> Vec<(f64, f64, &'a str)> {
    let mut placed: Vec<Rect> = avoid.to_vec();
    let mut out = Vec::new();
    for c in figs {
        let h = mul(eq_to_hor, c.label);
        if h[1] < 0.02 {
            continue;
        }
        let Some((x, y)) = cam.project(h) else {
            continue;
        };
        if !(0.0..cam.width).contains(&x) || !(0.0..cam.height).contains(&y) {
            continue;
        }
        // Upper case at 11 px: about 7.5 px a letter. If the label point is
        // taken, try a little below and above it before giving up.
        let half = 3.75 * c.name.chars().count() as f64 + 2.0;
        let spot = [0.0, 16.0, -16.0, 32.0, -32.0].into_iter().find_map(|dy| {
            let r = (x - half, y + dy - 7.0, x + half, y + dy + 7.0);
            (!placed.iter().any(|p| overlaps(&r, p))).then_some((r, y + dy))
        });
        let Some((r, y)) = spot else {
            continue;
        };
        placed.push(r);
        out.push((x, y, c.name.as_str()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{eq_to_horizon, vec_radec};
    use std::f64::consts::PI;

    const TINY: &[u8] = include_bytes!("../scripts/testdata/tiny_constellations.bin");

    #[test]
    fn decodes_the_shared_fixture() {
        // scripts/test_convert_constellations.py writes it.
        let figs = decode(TINY).unwrap();
        assert_eq!(figs.len(), 2);
        let abc = &figs[0];
        assert_eq!((abc.abbr.as_str(), abc.name.as_str()), ("Abc", "Alpha"));
        assert_eq!(abc.lines.len(), 2);
        assert!(!abc.lines[0].thin && abc.lines[1].thin);
        assert_eq!(abc.lines[0].points.len(), 3);
        let (ra, dec) = vec_radec(abc.lines[0].points[1]);
        assert!((ra.to_degrees() - 15.0).abs() < 1e-6 && (dec.to_degrees() - 10.0).abs() < 1e-6);
        let (ra, _) = vec_radec(figs[1].label);
        assert!(
            ra.to_degrees() < 1e-6 || ra.to_degrees() > 360.0 - 1e-6,
            "Zzz's label straddles 0h"
        );
    }

    #[test]
    fn rejects_bad_files() {
        assert!(decode(&TINY[..TINY.len() - 1]).is_err());
        let mut bad = TINY.to_vec();
        bad.push(0);
        assert!(decode(&bad).unwrap_err().contains("trailing"));
        let mut bad = TINY.to_vec();
        bad[0] = b'X';
        assert!(decode(&bad).is_err());
        let mut bad = TINY.to_vec();
        bad[4] = 9;
        assert!(decode(&bad).unwrap_err().contains("version"));
        let mut bad = TINY.to_vec();
        bad[8..12].copy_from_slice(&1000u32.to_le_bytes());
        assert!(decode(&bad).is_err(), "more entries than bytes");
        assert!(decode(b"").is_err());
    }

    #[test]
    fn the_committed_figures_decode() {
        let data = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/data/constellations.bin"
        ))
        .expect("read constellations.bin");
        let figs = decode(&data).unwrap();
        assert_eq!(figs.len(), 88);
        let ori = figs.iter().find(|c| c.abbr == "Ori").unwrap();
        assert_eq!(ori.name, "Orion");
        let (ra, dec) = vec_radec(ori.label);
        assert!((ra.to_degrees() / 15.0 - 5.5).abs() < 0.3 && dec.to_degrees().abs() < 15.0);
    }

    fn figure(points: &[(f64, f64)]) -> Constellation {
        let pts: Vec<[f64; 3]> = points
            .iter()
            .map(|(ra, dec)| radec_vec(ra.to_radians(), dec.to_radians()))
            .collect();
        Constellation {
            abbr: "Tst".into(),
            name: "Test".into(),
            label: pts[0],
            lines: vec![Line {
                thin: false,
                points: pts,
            }],
        }
    }

    #[test]
    fn segments_become_trimmed_curves_on_screen() {
        // Observer on the equator with RA 0h on the meridian: RA 0h Dec 0
        // is at the zenith. Look straight up.
        let m = eq_to_horizon(0.0, 0.0);
        let cam = SkyCamera::new(PI, 89.0f64.to_radians(), 100f64.to_radians());
        let fig = figure(&[(0.0, 0.0), (20.0, 0.0)]);
        let lines = screen_lines(std::slice::from_ref(&fig), &cam, &m);
        assert_eq!(lines.len(), 1);
        let run = &lines[0].1;
        assert!(run.len() > 5, "sampled along the arc");
        let (sx, sy) = cam.project(mul(&m, fig.lines[0].points[0])).unwrap();
        let gap = (run[0].0 - sx).hypot(run[0].1 - sy);
        assert!(
            (gap - GAP).abs() < 0.5,
            "trimmed {gap} px short of the star"
        );
        // Below the horizon: nothing. Half above: a run that stops there.
        let below = figure(&[(180.0, 0.0), (190.0, 0.0)]);
        assert!(screen_lines(&[below], &cam, &m).is_empty());
        let wide = SkyCamera::new(0.0, 0.0, 150f64.to_radians());
        let across = figure(&[(80.0, 0.0), (100.0, 0.0)]); // the west/east horizon
        for (_, run) in screen_lines(&[across], &wide, &m) {
            for p in run {
                let h = wide.pick(p.0, p.1).unwrap();
                assert!(h[1] > -0.01, "no part of a line below the horizon");
            }
        }
    }

    #[test]
    fn trim_keeps_long_runs_and_drops_short_ones() {
        let run = vec![(0.0, 0.0), (10.0, 0.0), (30.0, 0.0)];
        let t = trim(&run, 5.0).unwrap();
        assert_eq!(t.first(), Some(&(5.0, 0.0)));
        assert_eq!(t.last(), Some(&(25.0, 0.0)));
        assert_eq!(t.len(), 3, "the inner vertex stays");
        assert!(trim(&[(0.0, 0.0), (12.0, 0.0)], 5.0).is_none());
    }

    #[test]
    fn labels_only_above_the_horizon_and_on_screen() {
        let m = eq_to_horizon(0.0, 0.0);
        let cam = SkyCamera::new(PI, 89.0f64.to_radians(), 100f64.to_radians());
        let up = figure(&[(0.0, 0.0), (5.0, 0.0)]);
        let down = figure(&[(180.0, 0.0), (185.0, 0.0)]);
        let figs = [up, down];
        let l = labels(&figs, &cam, &m, &[]);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].2, "Test");
        // Not on top of a star name, nor of another constellation's name.
        let (x, y, _) = l[0];
        let moved = labels(&figs, &cam, &m, &[(x - 1.0, y - 1.0, x + 1.0, y + 1.0)]);
        assert_eq!(moved[0].1, y + 16.0, "moved below a star name in the way");
        let blocked = (x - 100.0, y - 100.0, x + 100.0, y + 100.0);
        assert!(labels(&figs, &cam, &m, &[blocked]).is_empty());
        let five: Vec<_> = (0..6).map(|_| figs[0].clone()).collect();
        assert_eq!(
            labels(&five, &cam, &m, &[]).len(),
            5,
            "five spots, then none"
        );
    }
}
