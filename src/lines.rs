//! Vector line geometry for coastlines and country borders.
//!
//! Browser-free: shapes in lon/lat degrees go in, flat `f32` segment lists
//! on the unit sphere come out. The renderer draws each segment as a
//! screen-space quad (see `shaders/line.vert`).
//!
//! * Geometry lives on the unit sphere (3D), so the ±180° seam and the poles
//!   need no special handling when drawing, and simplification measures real
//!   distances.
//! * Natural Earth cuts polygons along the antimeridian and the south pole,
//!   which leaves "seam edges" that are not coastlines. [`split_at_seam`]
//!   removes them before simplification.
//! * Level of detail: each layer is simplified with Douglas–Peucker at
//!   several tolerances at load time. The renderer picks the coarsest level
//!   whose error stays under about one screen pixel ([`lod_for`]).

use crate::geo::Shape;

/// Douglas–Peucker tolerances, in unit-sphere chord length (radians of arc,
/// to first order). Level 0 keeps every vertex.
pub const LOD_TOLERANCES: [f64; 4] = [0.0, 0.0003, 0.0012, 0.0025];

/// Pure geometry for one layer: flat segments, 6 floats per segment
/// (`ax ay az bx by bz`), for each level of detail.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LineLevels {
    pub levels: Vec<Vec<f32>>,
}

impl LineLevels {
    #[cfg(test)]
    pub fn segment_count(&self, level: usize) -> usize {
        self.levels.get(level).map_or(0, |l| l.len() / 6)
    }
}

/// Coastlines and borders, ready for upload.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LineSet {
    pub coast: LineLevels,
    pub borders: LineLevels,
}

/// World-space unit vector for a lon/lat in degrees. Same convention as the
/// shader: +Y north pole, longitude 0 on +Z, 90 E on +X.
pub fn to_unit(lon: f64, lat: f64) -> [f64; 3] {
    let (lo, la) = (lon.to_radians(), lat.to_radians());
    [la.cos() * lo.sin(), la.sin(), la.cos() * lo.cos()]
}

const SEAM_EPS: f64 = 1e-4;
/// Natural Earth's southern edge sits 0.0011 degrees short of the pole.
const POLE_EPS: f64 = 0.01;

/// True if the edge runs along the antimeridian or the pole rather than being
/// a real feature: both ends on lon = ±180, or both at lat = ±90.
pub fn is_seam_edge(a: [f64; 2], b: [f64; 2]) -> bool {
    let on_seam = |p: [f64; 2]| (p[0].abs() - 180.0).abs() < SEAM_EPS;
    let at_pole = |p: [f64; 2]| (p[1].abs() - 90.0).abs() < POLE_EPS;
    (on_seam(a) && on_seam(b)) || (at_pole(a) && at_pole(b))
}

/// Split a polyline (or ring, treated as open) into runs with seam edges
/// removed. Runs have at least two points.
pub fn split_at_seam(points: &[[f64; 2]]) -> Vec<Vec<[f64; 2]>> {
    let mut runs = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for w in points.windows(2) {
        if is_seam_edge(w[0], w[1]) {
            if cur.len() >= 2 {
                runs.push(std::mem::take(&mut cur));
            }
            cur.clear();
            continue;
        }
        if cur.is_empty() {
            cur.push(w[0]);
        }
        cur.push(w[1]);
    }
    if cur.len() >= 2 {
        runs.push(cur);
    }
    runs
}

fn dist_point_segment(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    let t = if len2 > 0.0 {
        ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let d = [ap[0] - t * ab[0], ap[1] - t * ab[1], ap[2] - t * ab[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Douglas–Peucker on 3D points. Returns the indices to keep (ascending,
/// always including the first and last). Iterative, so long coastlines
/// cannot overflow the stack. A tolerance of 0 keeps every point.
pub fn simplify(points: &[[f64; 3]], tol: f64) -> Vec<usize> {
    let n = points.len();
    if n <= 2 || tol <= 0.0 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((lo, hi)) = stack.pop() {
        if hi <= lo + 1 {
            continue;
        }
        let (mut worst, mut idx) = (0.0, lo);
        for i in lo + 1..hi {
            let d = dist_point_segment(points[i], points[lo], points[hi]);
            if d > worst {
                (worst, idx) = (d, i);
            }
        }
        if worst > tol {
            keep[idx] = true;
            stack.push((lo, idx));
            stack.push((idx, hi));
        }
    }
    (0..n).filter(|&i| keep[i]).collect()
}

/// Build flat segments for shapes at one tolerance: seam edges removed,
/// each run simplified, zero-length segments dropped.
pub fn build_segments(shapes: &[Shape], tol: f64) -> Vec<f32> {
    let mut out = Vec::new();
    for shape in shapes {
        for run in split_at_seam(&shape.points) {
            let pts: Vec<[f64; 3]> = run.iter().map(|p| to_unit(p[0], p[1])).collect();
            let keep = simplify(&pts, tol);
            for w in keep.windows(2) {
                let (a, b) = (pts[w[0]], pts[w[1]]);
                if a == b {
                    continue;
                }
                out.extend([a[0], a[1], a[2], b[0], b[1], b[2]].map(|v| v as f32));
            }
        }
    }
    out
}

/// All levels of detail for one layer.
pub fn build_levels(shapes: &[Shape]) -> LineLevels {
    LineLevels {
        levels: LOD_TOLERANCES
            .iter()
            .map(|&t| build_segments(shapes, t))
            .collect(),
    }
}

/// Coarsest level whose simplification error stays under about one screen
/// pixel. `alt` is the camera altitude in globe radii, `min_px` the shorter
/// viewport side in device pixels, `tan_half_fov` the camera constant.
/// One pixel at the globe centre spans `2 * alt * tan_half_fov / min_px`
/// radii; the tolerance may use half of that (error is measured one-sided).
pub fn lod_for(alt: f64, min_px: f64, tan_half_fov: f64) -> usize {
    if !(alt.is_finite() && min_px.is_finite()) || min_px <= 0.0 {
        return 0;
    }
    let half_px = alt.max(0.0) * tan_half_fov / min_px;
    LOD_TOLERANCES
        .iter()
        .rposition(|&t| t <= half_px)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(points: &[[f64; 2]], closed: bool) -> Shape {
        Shape {
            points: points.to_vec(),
            closed,
        }
    }

    fn len(p: [f64; 3]) -> f64 {
        (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
    }

    #[test]
    fn unit_vectors_match_shader_convention() {
        let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-12);
        assert!(close(to_unit(0.0, 0.0), [0.0, 0.0, 1.0]));
        assert!(close(to_unit(90.0, 0.0), [1.0, 0.0, 0.0]));
        assert!(close(to_unit(0.0, 90.0), [0.0, 1.0, 0.0]));
        // Both sides of the seam are the same point.
        assert!(close(to_unit(180.0, 30.0), to_unit(-180.0, 30.0)));
        // Every longitude collapses to one point at the pole.
        assert!(close(to_unit(-135.0, -90.0), to_unit(77.0, -90.0)));
        assert!((len(to_unit(12.3, -45.6)) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn seam_edges_are_detected() {
        assert!(is_seam_edge([180.0, 65.0], [180.0, 68.0]));
        assert!(is_seam_edge([-180.0, 65.0], [180.0, 68.0]));
        assert!(is_seam_edge([-180.0, -90.0], [180.0, -90.0]));
        assert!(is_seam_edge([10.0, -90.0], [50.0, -90.0]));
        assert!(!is_seam_edge([180.0, 65.0], [179.0, 65.0]));
        assert!(!is_seam_edge([0.0, 0.0], [1.0, 1.0]));
        assert!(!is_seam_edge([180.0, 65.0], [-170.0, 66.0]));
    }

    #[test]
    fn split_removes_only_seam_edges() {
        // Chukotka-like ring: coast, then a run down the antimeridian.
        let ring = [
            [170.0, 66.0],
            [180.0, 65.0],
            [180.0, 68.0],
            [170.0, 69.0],
            [170.0, 66.0],
        ];
        let runs = split_at_seam(&ring);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0], vec![[170.0, 66.0], [180.0, 65.0]]);
        assert_eq!(runs[1], vec![[180.0, 68.0], [170.0, 69.0], [170.0, 66.0]]);
        // No seam: one run, unchanged.
        let line = [[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]];
        assert_eq!(split_at_seam(&line), vec![line.to_vec()]);
        // Only seam edges: nothing left.
        assert!(split_at_seam(&[[180.0, 0.0], [180.0, 10.0], [180.0, 20.0]]).is_empty());
        assert!(split_at_seam(&[]).is_empty());
        assert!(split_at_seam(&[[1.0, 1.0]]).is_empty());
    }

    #[test]
    fn antarctica_pole_edge_is_dropped() {
        let ring = shape(
            &[
                [-180.0, -78.0],
                [-150.0, -80.0],
                [180.0, -78.0],
                [180.0, -90.0],
                [-180.0, -90.0],
                [-180.0, -78.0],
            ],
            true,
        );
        // Coast from -180 to 180 along the barrier, minus the pole and seam edges.
        let seg = build_segments(&[ring], 0.0);
        assert_eq!(seg.len() / 6, 2);
    }

    #[test]
    fn simplify_keeps_endpoints_and_corners() {
        let pts: Vec<[f64; 3]> = [[0.0, 0.0], [0.5, 0.00001], [1.0, 0.0], [1.0, 1.0]]
            .iter()
            .map(|p| to_unit(p[0], p[1]))
            .collect();
        assert_eq!(simplify(&pts, 0.0), vec![0, 1, 2, 3]);
        // The nearly collinear point goes; the corner stays.
        assert_eq!(simplify(&pts, 0.001), vec![0, 2, 3]);
        assert_eq!(simplify(&pts, 10.0), vec![0, 3]);
        assert_eq!(simplify(&pts[..2], 1.0), vec![0, 1]);
        assert!(simplify(&[], 1.0).is_empty());
    }

    #[test]
    fn simplify_error_stays_within_tolerance() {
        // A wiggly line; every dropped point must lie within tol of the result.
        let pts: Vec<[f64; 3]> = (0..500)
            .map(|i| {
                let t = i as f64;
                to_unit(
                    t * 0.05,
                    20.0 + (t * 0.3).sin() * 0.4 + (t * 1.7).sin() * 0.05,
                )
            })
            .collect();
        for &tol in &LOD_TOLERANCES[1..] {
            let keep = simplify(&pts, tol);
            assert!(keep.len() < pts.len());
            for w in keep.windows(2) {
                for p in &pts[w[0]..=w[1]] {
                    assert!(dist_point_segment(*p, pts[w[0]], pts[w[1]]) <= tol + 1e-12);
                }
            }
        }
    }

    #[test]
    fn simplify_survives_long_input_and_duplicates() {
        let pts: Vec<[f64; 3]> = (0..200_000)
            .map(|i| to_unit((i % 360) as f64 - 180.0, ((i / 360) % 170) as f64 - 85.0))
            .collect();
        assert!(simplify(&pts, 0.01).len() >= 2);
        let same = vec![to_unit(1.0, 1.0); 10];
        assert_eq!(simplify(&same, 0.1), vec![0, 9]);
        // Duplicate points produce no segments at all.
        let s = shape(&[[1.0, 1.0], [1.0, 1.0], [1.0, 1.0]], false);
        assert!(build_segments(&[s], 0.0).is_empty());
    }

    #[test]
    fn segments_across_the_seam_stay_short() {
        // A real line crossing the antimeridian (179 -> -179) is 2 degrees of
        // arc, not 358: in 3D there is no wrap to get wrong.
        let s = shape(&[[179.0, 10.0], [-179.0, 10.0]], false);
        let seg = build_segments(&[s], 0.0);
        assert_eq!(seg.len(), 6);
        let d: f64 = (0..3)
            .map(|i| f64::from(seg[i] - seg[i + 3]).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(d < 0.04, "chord {d}");
        // Vertices are on the unit sphere.
        let a = [seg[0], seg[1], seg[2]].map(f64::from);
        assert!((len(a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn lod_gets_coarser_with_altitude_and_smaller_screens() {
        let t = 0.364;
        let near = lod_for(0.06, 800.0, t);
        let mid = lod_for(0.25, 800.0, t);
        let far = lod_for(6.0, 800.0, t);
        assert_eq!(near, 0, "400 km on a laptop uses full detail");
        assert!(mid <= far);
        assert_eq!(far, LOD_TOLERANCES.len() - 1);
        // A smaller viewport has bigger pixels, so it can simplify more.
        assert!(lod_for(0.25, 300.0, t) >= mid);
        // Garbage never panics and falls back to full detail.
        assert_eq!(lod_for(f64::NAN, 800.0, t), 0);
        assert_eq!(lod_for(1.0, 0.0, t), 0);
        assert_eq!(lod_for(-1.0, 800.0, t), 0);
    }

    #[test]
    fn bundled_data_builds_and_shrinks_with_level() {
        let coast_bin: &[u8] = include_bytes!("../assets/data/ne_50m_land.bin");
        let lakes_bin: &[u8] = include_bytes!("../assets/data/ne_50m_lakes.bin");
        let border_bin: &[u8] =
            include_bytes!("../assets/data/ne_50m_admin_0_boundary_lines_land.bin");
        let mut coast = crate::geo::decode_shapes(coast_bin).unwrap();
        coast.extend(crate::geo::decode_shapes(lakes_bin).unwrap());
        let borders = crate::geo::decode_shapes(border_bin).unwrap();
        for shapes in [&coast, &borders] {
            let lv = build_levels(shapes);
            assert_eq!(lv.levels.len(), LOD_TOLERANCES.len());
            let counts: Vec<usize> = (0..lv.levels.len()).map(|i| lv.segment_count(i)).collect();
            println!("segments per level: {counts:?}");
            assert!(counts[0] > 1000);
            assert!(counts.windows(2).all(|w| w[1] < w[0]), "{counts:?}");
            // Everything lies on the unit sphere and is finite.
            for v in lv.levels[0].chunks(3) {
                let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                assert!((l - 1.0).abs() < 1e-5);
            }
        }
        // No coast segment may run along the antimeridian or the pole.
        let seg = build_segments(&coast, 0.0);
        for s in seg.chunks(6) {
            let on_pole = |y: f32| y.abs() > 0.999_999_9;
            assert!(!(on_pole(s[1]) && on_pole(s[4])), "pole edge survived");
        }
    }
}
