//! Convex brushes: solids bounded by planes (ramps, wedges, any convex shape).
//! Boxes stay `Aabb` (their trace path is unchanged); convex brushes add a
//! separate path. Each brush also carries axial **bevel** planes (its bounding
//! box) so a swept player box doesn't catch on edges; bevels aren't drawn.

use crate::math::Vec3;
use crate::world::Aabb;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    /// Unit outward normal.
    pub normal: Vec3,
    /// Points p with dot(p, normal) <= dist are inside.
    pub dist: f32,
    /// Collision-only helper plane (not a real face).
    pub bevel: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConvexBrush {
    pub planes: Vec<Plane>,
    pub bounds: Aabb,
    pub color: [f32; 3],
    /// Visual material tag (themes only; ignored by collision and the content hash).
    pub material: Option<String>,
}

/// Which way a ramp goes up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rise {
    PosX,
    NegX,
    PosY,
    NegY,
}

impl Rise {
    pub fn parse(s: &str) -> Option<Rise> {
        Some(match s {
            "+x" => Rise::PosX,
            "-x" => Rise::NegX,
            "+y" => Rise::PosY,
            "-y" => Rise::NegY,
            _ => return None,
        })
    }
}

const EPS: f64 = 1e-3;

/// Intersection of three planes (None if nearly parallel).
fn intersect(a: &Plane, b: &Plane, c: &Plane) -> Option<[f64; 3]> {
    let v = |p: &Plane| [p.normal.x as f64, p.normal.y as f64, p.normal.z as f64];
    let (n1, n2, n3) = (v(a), v(b), v(c));
    let cross = |x: [f64; 3], y: [f64; 3]| {
        [
            x[1] * y[2] - x[2] * y[1],
            x[2] * y[0] - x[0] * y[2],
            x[0] * y[1] - x[1] * y[0],
        ]
    };
    let dot = |x: [f64; 3], y: [f64; 3]| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
    let c23 = cross(n2, n3);
    let det = dot(n1, c23);
    if det.abs() < 1e-9 {
        return None;
    }
    let (c31, c12) = (cross(n3, n1), cross(n1, n2));
    let (d1, d2, d3) = (a.dist as f64, b.dist as f64, c.dist as f64);
    Some([0, 1, 2].map(|i| (d1 * c23[i] + d2 * c31[i] + d3 * c12[i]) / det))
}

impl ConvexBrush {
    /// Build from face planes (normal need not be unit; it is normalised).
    /// Fails if the planes don't enclose a non-empty bounded volume.
    pub fn from_planes(faces: &[(Vec3, f32)], color: [f32; 3]) -> Result<ConvexBrush, String> {
        let mut planes = Vec::with_capacity(faces.len() + 6);
        for (n, d) in faces {
            let len = n.length();
            if !len.is_finite() || len < 1e-6 || !d.is_finite() {
                return Err("degenerate plane".into());
            }
            planes.push(Plane {
                normal: *n * (1.0 / len),
                dist: d / len,
                bevel: false,
            });
        }
        let mut b = ConvexBrush {
            planes,
            bounds: Aabb::new(Vec3::ZERO, Vec3::ZERO),
            color,
            material: None,
        };
        let verts = b.vertices();
        if verts.len() < 4 {
            return Err("planes don't enclose a volume".into());
        }
        let mut min = verts[0];
        let mut max = verts[0];
        for v in &verts {
            min = Vec3::new(min.x.min(v.x), min.y.min(v.y), min.z.min(v.z));
            max = Vec3::new(max.x.max(v.x), max.y.max(v.y), max.z.max(v.z));
        }
        if (0..3).any(|a| max.component(a) - min.component(a) < 1e-3)
            || (0..3).any(|a| max.component(a).abs() > 1e6)
        {
            return Err("brush is flat or unbounded".into());
        }
        b.bounds = Aabb::new(min, max);
        // Axial bevels (skip any that coincide with a real face).
        let axial = [
            (Vec3::new(1.0, 0.0, 0.0), max.x),
            (Vec3::new(-1.0, 0.0, 0.0), -min.x),
            (Vec3::new(0.0, 1.0, 0.0), max.y),
            (Vec3::new(0.0, -1.0, 0.0), -min.y),
            (Vec3::new(0.0, 0.0, 1.0), max.z),
            (Vec3::new(0.0, 0.0, -1.0), -min.z),
        ];
        for (n, d) in axial {
            if !b.planes.iter().any(|p| p.normal.dot(n) > 0.9999) {
                b.planes.push(Plane {
                    normal: n,
                    dist: d,
                    bevel: true,
                });
            }
        }
        Ok(b)
    }

    /// Wedge filling `min..max` whose top slopes up from `min.z` at the low
    /// end to `max.z` at the high end in direction `rise`.
    pub fn ramp(min: Vec3, max: Vec3, rise: Rise, color: [f32; 3]) -> Result<ConvexBrush, String> {
        let h = max.z - min.z;
        let x = |v: f32| Vec3::new(v, 0.0, 0.0);
        let y = |v: f32| Vec3::new(0.0, v, 0.0);
        let (slope, high_face) = match rise {
            // Slope plane through the low bottom edge and the high top edge.
            Rise::PosX => (Vec3::new(-h, 0.0, max.x - min.x), (x(1.0), max.x)),
            Rise::NegX => (Vec3::new(h, 0.0, max.x - min.x), (x(-1.0), -min.x)),
            Rise::PosY => (Vec3::new(0.0, -h, max.y - min.y), (y(1.0), max.y)),
            Rise::NegY => (Vec3::new(0.0, h, max.y - min.y), (y(-1.0), -min.y)),
        };
        let low_corner = match rise {
            Rise::PosX | Rise::PosY => min,
            Rise::NegX => Vec3::new(max.x, min.y, min.z),
            Rise::NegY => Vec3::new(min.x, max.y, min.z),
        };
        let mut faces = vec![
            (Vec3::new(0.0, 0.0, -1.0), -min.z),
            (slope, slope.dot(low_corner)),
            high_face,
        ];
        match rise {
            Rise::PosX | Rise::NegX => faces.extend([(y(1.0), max.y), (y(-1.0), -min.y)]),
            Rise::PosY | Rise::NegY => faces.extend([(x(1.0), max.x), (x(-1.0), -min.x)]),
        }
        ConvexBrush::from_planes(&faces, color)
    }

    /// Corner points of the brush.
    pub fn vertices(&self) -> Vec<Vec3> {
        let p = &self.planes;
        let mut out: Vec<Vec3> = Vec::new();
        for i in 0..p.len() {
            for j in i + 1..p.len() {
                for k in j + 1..p.len() {
                    let Some(v) = intersect(&p[i], &p[j], &p[k]) else {
                        continue;
                    };
                    let inside = p.iter().all(|q| {
                        q.normal.x as f64 * v[0]
                            + q.normal.y as f64 * v[1]
                            + q.normal.z as f64 * v[2]
                            - q.dist as f64
                            <= EPS
                    });
                    let v = Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32);
                    if inside && !out.iter().any(|o| (*o - v).length() < 1e-2) {
                        out.push(v);
                    }
                }
            }
        }
        out
    }

    /// Polygon of face `i` (vertices in order around the face, counter-clockwise
    /// seen from outside), or empty for bevels / degenerate faces.
    pub fn face(&self, i: usize) -> Vec<Vec3> {
        let pl = self.planes[i];
        if pl.bevel {
            return vec![];
        }
        let verts: Vec<Vec3> = self
            .vertices()
            .into_iter()
            .filter(|v| ((v.dot(pl.normal) - pl.dist) as f64).abs() < 0.05)
            .collect();
        if verts.len() < 3 {
            return vec![];
        }
        let c = verts.iter().fold(Vec3::ZERO, |a, v| a + *v) * (1.0 / verts.len() as f32);
        let n = pl.normal;
        let u = if n.z.abs() < 0.9 {
            n.cross(Vec3::new(0.0, 0.0, 1.0))
        } else {
            n.cross(Vec3::new(1.0, 0.0, 0.0))
        }
        .normalize();
        let w = n.cross(u);
        let mut tagged: Vec<(f64, Vec3)> = verts
            .into_iter()
            .map(|v| {
                let d = v - c;
                ((d.dot(w) as f64).atan2(d.dot(u) as f64), v)
            })
            .collect();
        tagged.sort_by(|a, b| a.0.total_cmp(&b.0));
        tagged.into_iter().map(|t| t.1).collect()
    }

    /// Is the point inside (or on) the brush?
    pub fn contains(&self, p: Vec3) -> bool {
        self.planes.iter().all(|q| p.dot(q.normal) - q.dist <= 1e-3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_geometry() {
        let r = ConvexBrush::ramp(
            Vec3::new(0.0, -64.0, 0.0),
            Vec3::new(128.0, 64.0, 64.0),
            Rise::PosX,
            [1.0; 3],
        )
        .unwrap();
        let v = r.vertices();
        assert_eq!(v.len(), 6, "{v:?}");
        assert_eq!(
            r.bounds,
            Aabb::new(Vec3::new(0.0, -64.0, 0.0), Vec3::new(128.0, 64.0, 64.0))
        );
        // 5 real faces + top/-x bevels (top and the low end are not faces of a wedge).
        assert_eq!(r.planes.iter().filter(|p| !p.bevel).count(), 5);
        assert_eq!(r.planes.iter().filter(|p| p.bevel).count(), 2);
        // Slope normal points up and back (-x).
        let slope = r.planes[1].normal;
        assert!(slope.z > 0.0 && slope.x < 0.0 && (slope.length() - 1.0).abs() < 1e-6);
        assert!(r.contains(Vec3::new(100.0, 0.0, 10.0)));
        assert!(
            !r.contains(Vec3::new(10.0, 0.0, 30.0)),
            "above the slope near the low end"
        );
        // Faces: bottom quad, slope quad, high face quad, two side triangles.
        let sizes: Vec<usize> = (0..r.planes.len()).map(|i| r.face(i).len()).collect();
        assert_eq!(sizes, vec![4, 4, 4, 3, 3, 0, 0]);
    }

    #[test]
    fn all_rise_directions_are_valid() {
        for rise in [Rise::PosX, Rise::NegX, Rise::PosY, Rise::NegY] {
            let r = ConvexBrush::ramp(
                Vec3::new(-50.0, -50.0, 0.0),
                Vec3::new(50.0, 50.0, 40.0),
                rise,
                [1.0; 3],
            )
            .unwrap();
            assert_eq!(r.vertices().len(), 6, "{rise:?}");
            let top = r.planes[1].normal;
            assert!(top.z > 0.9, "{rise:?} slope normal {top:?}");
        }
    }

    #[test]
    fn rejects_unbounded_or_empty() {
        assert!(ConvexBrush::from_planes(&[(Vec3::new(0.0, 0.0, 1.0), 0.0)], [1.0; 3]).is_err());
        let contradictory = [
            (Vec3::new(1.0, 0.0, 0.0), -10.0),
            (Vec3::new(-1.0, 0.0, 0.0), -10.0),
            (Vec3::new(0.0, 1.0, 0.0), 1.0),
            (Vec3::new(0.0, -1.0, 0.0), 1.0),
            (Vec3::new(0.0, 0.0, 1.0), 1.0),
            (Vec3::new(0.0, 0.0, -1.0), 1.0),
        ];
        assert!(ConvexBrush::from_planes(&contradictory, [1.0; 3]).is_err());
    }
}
