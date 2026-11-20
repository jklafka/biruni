//! The sky view's star catalog: decoding `stars.bin` (written by
//! `scripts/convert_stars.py` from the HYG database, CC BY-SA 4.0; the
//! format is described there), star colors, how faint a star the view
//! shows, and which bright stars get a name label. Browser-free.

use crate::sky::{mul, radec_vec, Mat3, SkyCamera};
use std::f64::consts::{FRAC_PI_2, TAU};

const MAGIC: &[u8; 4] = b"STAR";
const VERSION: u8 = 1;
const HEADER: usize = 16;
const RECORD: usize = 10;
const NO_CI: i8 = -128;
/// Faintest stars in the file (`MAG_LIMIT` in the converter).
pub const CATALOG_LIMIT: f32 = 8.0;
/// Floats per star in the vertex buffer: direction (3), magnitude, color (3).
pub const STRIDE: usize = 7;

#[derive(Clone, Debug, PartialEq)]
pub struct Star {
    /// J2000 right ascension and declination, radians.
    pub ra: f64,
    pub dec: f64,
    pub mag: f32,
    /// B-V color index, if known.
    pub ci: Option<f32>,
}

#[derive(Debug, Default)]
pub struct Catalog {
    /// Brightest first.
    pub stars: Vec<Star>,
    /// Proper names: (star index, name), in star order.
    pub names: Vec<(usize, String)>,
}

fn u32_at(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}

pub fn decode(data: &[u8]) -> Result<Catalog, String> {
    if data.len() < HEADER || &data[..4] != MAGIC {
        return Err("stars.bin: not a star catalog".into());
    }
    if data[4] != VERSION {
        return Err(format!("stars.bin: unsupported version {}", data[4]));
    }
    let n = u32_at(data, 8) as usize;
    let k = u32_at(data, 12) as usize;
    let end = n
        .checked_mul(RECORD)
        .and_then(|b| b.checked_add(HEADER))
        .filter(|&e| e <= data.len())
        .ok_or("stars.bin: truncated star records")?;
    let mut stars = Vec::with_capacity(n);
    for i in 0..n {
        let at = HEADER + i * RECORD;
        let ra = u32_at(data, at) as f64 / 4_294_967_296.0 * TAU;
        let dec = u32_at(data, at + 4) as i32 as f64 / i32::MAX as f64 * FRAC_PI_2;
        let mag = data[at + 8] as f32 / 20.0 - 2.0;
        let ci = data[at + 9] as i8;
        stars.push(Star {
            ra,
            dec,
            mag,
            ci: (ci != NO_CI).then_some(ci as f32 / 50.0),
        });
    }
    let mut names = Vec::with_capacity(k);
    let mut at = end;
    for _ in 0..k {
        if at + 5 > data.len() {
            return Err("stars.bin: truncated name table".into());
        }
        let i = u32_at(data, at) as usize;
        let len = data[at + 4] as usize;
        at += 5;
        let bytes = data
            .get(at..at + len)
            .filter(|_| i < n)
            .ok_or("stars.bin: bad name entry")?;
        let name = std::str::from_utf8(bytes).map_err(|_| "stars.bin: name is not UTF-8")?;
        names.push((i, name.to_string()));
        at += len;
    }
    if at != data.len() {
        return Err("stars.bin: trailing bytes".into());
    }
    Ok(Catalog { stars, names })
}

/// Display color for a B-V color index: blue-white for hot stars through
/// white to orange-red for cool ones (a smoothed version of the blackbody
/// colors of Mitchell Charity's table). Unknown is a neutral white.
pub fn bv_color(ci: Option<f32>) -> [f32; 3] {
    const STOPS: [(f32, [f32; 3]); 7] = [
        (-0.4, [0.62, 0.72, 1.00]),
        (0.0, [0.80, 0.86, 1.00]),
        (0.4, [0.98, 0.98, 1.00]),
        (0.65, [1.00, 0.95, 0.86]),
        (1.0, [1.00, 0.86, 0.68]),
        (1.5, [1.00, 0.76, 0.52]),
        (2.0, [1.00, 0.66, 0.40]),
    ];
    let c = ci.filter(|c| c.is_finite()).unwrap_or(0.5);
    if c <= STOPS[0].0 {
        return STOPS[0].1;
    }
    for w in STOPS.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if c <= b {
            let t = (c - a) / (b - a);
            return [0, 1, 2].map(|i| ca[i] + (cb[i] - ca[i]) * t);
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// Faintest magnitude drawn at a field of view: about the naked-eye 6 at
/// 100 degrees, fainter as the view narrows (as through binoculars), up to
/// the catalog's limit.
pub fn limiting_magnitude(fov: f64) -> f32 {
    let m = 6.0 + 2.2 * (100f64.to_radians() / fov.max(1e-6)).log10();
    (m as f32).clamp(5.0, CATALOG_LIMIT)
}

/// Number of stars at or brighter than `mag` (the catalog is sorted).
pub fn count_brighter(cat: &Catalog, mag: f32) -> usize {
    cat.stars.partition_point(|s| s.mag <= mag)
}

/// Interleaved vertex data, [`STRIDE`] floats per star: equatorial unit
/// vector, magnitude, color.
pub fn vertices(cat: &Catalog) -> Vec<f32> {
    let mut out = Vec::with_capacity(cat.stars.len() * STRIDE);
    for s in &cat.stars {
        let v = radec_vec(s.ra, s.dec);
        out.extend([v[0] as f32, v[1] as f32, v[2] as f32, s.mag]);
        out.extend(bv_color(s.ci));
    }
    out
}

/// Screen box (x0, y0, x1, y1) of a star name drawn to the right of the
/// star at `(x, y)`.
pub fn label_rect(x: f64, y: f64, name: &str) -> Rect {
    let w = 7.0 * name.chars().count() as f64;
    (x + 6.0, y - 8.0, x + 8.0 + w, y + 8.0)
}

/// An axis-aligned screen box: x0, y0, x1, y1.
pub type Rect = (f64, f64, f64, f64);

pub fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// Brightest stars get names: those at least this many magnitudes brighter
/// than the view's limit.
const LABEL_MARGIN: f32 = 3.5;
const MAX_LABELS: usize = 30;

/// Name labels for the brightest named stars in view, above the horizon
/// and clear of each other and of `avoid`: (x, y, name) at the star, CSS
/// pixels. `mag_limit` is the faintest magnitude drawn.
pub fn labels<'a>(
    cat: &'a Catalog,
    cam: &SkyCamera,
    eq_to_hor: &Mat3,
    mag_limit: f32,
    avoid: &[Rect],
) -> Vec<(f64, f64, &'a str)> {
    let limit = mag_limit - LABEL_MARGIN;
    let mut placed: Vec<Rect> = avoid.to_vec();
    let mut out = Vec::new();
    for (i, name) in &cat.names {
        let s = &cat.stars[*i];
        if s.mag > limit {
            break; // names are in star order, brightest first
        }
        let h = mul(eq_to_hor, radec_vec(s.ra, s.dec));
        // Not on the horizon line itself (about 1.7 degrees up).
        if h[1] < 0.03 {
            continue;
        }
        let Some((x, y)) = cam.project(h) else {
            continue;
        };
        if !(0.0..cam.width).contains(&x) || !(0.0..cam.height).contains(&y) {
            continue;
        }
        let r = label_rect(x, y, name);
        if placed.iter().any(|p| overlaps(&r, p)) {
            continue;
        }
        placed.push(r);
        out.push((x, y, name.as_str()));
        if out.len() == MAX_LABELS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::eq_to_horizon;
    use std::f64::consts::PI;

    const TINY: &[u8] = include_bytes!("../scripts/testdata/tiny_stars.bin");

    #[test]
    fn decodes_the_shared_fixture() {
        // scripts/test_convert_stars.py writes it: four stars, Vega named.
        let cat = decode(TINY).unwrap();
        assert_eq!(cat.stars.len(), 4);
        assert_eq!(cat.names, vec![(1, "Vega".to_string())]);
        let vega = &cat.stars[1];
        assert!((vega.ra.to_degrees() / 15.0 - 18.615649).abs() < 1e-6);
        assert!((vega.dec.to_degrees() - 38.783692).abs() < 1e-6);
        assert!((vega.mag - 0.05).abs() < 1e-6);
        assert_eq!(vega.ci, Some(0.0));
        assert_eq!(cat.stars[2].ci, None, "unknown color");
        assert!((cat.stars[2].dec.to_degrees() + 89.5).abs() < 1e-6);
        assert!(cat.stars.windows(2).all(|w| w[0].mag <= w[1].mag));
    }

    #[test]
    fn rejects_bad_files() {
        assert!(decode(&TINY[..TINY.len() - 1]).is_err());
        assert!(decode(&TINY[..20]).is_err());
        assert!(decode(b"").is_err());
        let mut bad = TINY.to_vec();
        bad[0] = b'X';
        assert!(decode(&bad).is_err());
        let mut bad = TINY.to_vec();
        bad[4] = 2;
        assert!(decode(&bad).unwrap_err().contains("version"));
        let mut bad = TINY.to_vec();
        bad.push(0);
        assert!(decode(&bad).unwrap_err().contains("trailing"));
        let mut bad = TINY.to_vec();
        bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&bad).is_err(), "huge count");
        let mut bad = TINY.to_vec();
        let name_at = HEADER + 4 * RECORD;
        bad[name_at..name_at + 4].copy_from_slice(&9u32.to_le_bytes());
        assert!(decode(&bad).is_err(), "name for a star that is not there");
    }

    #[test]
    fn the_committed_catalog_decodes() {
        let data = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/data/stars.bin"
        ))
        .expect("read stars.bin");
        let cat = decode(&data).unwrap();
        assert!(cat.stars.len() > 30_000);
        let sirius = cat.names.iter().find(|(_, n)| n == "Sirius").unwrap().0;
        assert_eq!(sirius, 0, "the brightest star");
        assert_eq!(count_brighter(&cat, CATALOG_LIMIT), cat.stars.len());
        assert!(count_brighter(&cat, 6.5) > 8000 && count_brighter(&cat, 6.5) < 10_000);
    }

    #[test]
    fn colors_run_from_blue_to_red() {
        let hot = bv_color(Some(-0.3));
        let sun = bv_color(Some(0.65));
        let cool = bv_color(Some(1.8));
        assert!(hot[2] > hot[0] && cool[0] > cool[2]);
        assert_eq!(sun, [1.00, 0.95, 0.86]);
        assert_eq!(bv_color(Some(-5.0)), bv_color(Some(-0.4)));
        assert_eq!(bv_color(Some(9.0)), bv_color(Some(2.0)));
        assert_eq!(bv_color(None), bv_color(Some(0.5)));
        assert_eq!(bv_color(Some(f32::NAN)), bv_color(None));
        for c in [hot, sun, cool] {
            assert!(c.iter().all(|v| (0.0..=1.0).contains(v)));
        }
    }

    #[test]
    fn narrower_views_show_fainter_stars() {
        assert!((limiting_magnitude(100f64.to_radians()) - 6.0).abs() < 1e-6);
        assert!(limiting_magnitude(20f64.to_radians()) > 7.0);
        assert_eq!(limiting_magnitude(1f64.to_radians()), CATALOG_LIMIT);
        assert!((limiting_magnitude(150f64.to_radians()) - 5.6126).abs() < 1e-3);
        let nan = limiting_magnitude(f64::NAN);
        assert!((5.0..=CATALOG_LIMIT).contains(&nan));
    }

    #[test]
    fn vertices_hold_direction_magnitude_and_color() {
        let cat = decode(TINY).unwrap();
        let v = vertices(&cat);
        assert_eq!(v.len(), 4 * STRIDE);
        let vega = &v[STRIDE..2 * STRIDE];
        let want = radec_vec(cat.stars[1].ra, cat.stars[1].dec);
        assert!((vega[0] as f64 - want[0]).abs() < 1e-6 && (vega[1] as f64 - want[1]).abs() < 1e-6);
        assert!((vega[3] - 0.05).abs() < 1e-6);
        assert_eq!(&vega[4..7], &bv_color(Some(0.0)));
    }

    #[test]
    fn labels_name_bright_stars_in_view_without_overlap() {
        let cat = Catalog {
            stars: vec![
                Star {
                    ra: 0.0,
                    dec: 0.0,
                    mag: 0.0,
                    ci: None,
                },
                Star {
                    ra: 0.001,
                    dec: 0.0,
                    mag: 0.5,
                    ci: None,
                },
                Star {
                    ra: 0.3,
                    dec: 0.2,
                    mag: 1.0,
                    ci: None,
                },
                Star {
                    ra: PI,
                    dec: 0.0,
                    mag: 1.0,
                    ci: None,
                },
                Star {
                    ra: 0.2,
                    dec: -0.1,
                    mag: 5.0,
                    ci: None,
                },
            ],
            names: vec![
                (0, "Bright".into()),
                (1, "Twin".into()),
                (2, "Other".into()),
                (3, "Behind".into()),
                (4, "Faint".into()),
            ],
        };
        // At the equator with RA 0h on the meridian, RA 0h Dec 0 is due
        // south at altitude 90: look straight up.
        let m = eq_to_horizon(0.0, 0.0);
        let h = mul(&m, radec_vec(0.0, 0.0));
        assert!(h[1] > 0.999);
        let cam = SkyCamera::new(PI, 89.0f64.to_radians(), 100f64.to_radians());
        let limit = limiting_magnitude(cam.fov);
        let names: Vec<&str> = labels(&cat, &cam, &m, limit, &[])
            .iter()
            .map(|l| l.2)
            .collect();
        assert_eq!(
            names,
            vec!["Bright", "Other"],
            "twin overlaps, behind is below the horizon, faint is too faint"
        );
    }
}
