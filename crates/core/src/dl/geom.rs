//! Display-list geometry. All coordinates are in **points** (1/72 inch) in slide space,
//! origin top-left, y down. Points — not pixels — are what makes the display list
//! resolution-independent: zoom and device-pixel-ratio are applied by the renderer's
//! view transform, so changing either re-renders without re-laying-out.

use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    #[inline]
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    #[inline]
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    #[inline]
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
    /// The rectangle that contains both, treating empty rects as absent.
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }
}

/// A 2D affine transform laid out as the six values Canvas2D's `setTransform` wants:
/// `[a c e; b d f]`, i.e. `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub const fn translate(tx: f32, ty: f32) -> Self {
        Transform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    pub const fn scale(sx: f32, sy: f32) -> Self {
        Transform {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn rotate(radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        Transform {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Rotation about an arbitrary centre — the form every OOXML `<a:xfrm rot>` needs,
    /// since shapes rotate about the centre of their bounding box.
    ///
    /// Move the centre to the origin *first*, then rotate, then move it back. Composing
    /// these the other way round rotates about the origin and translates afterwards,
    /// which sends the shape off across the slide.
    pub fn rotate_about(radians: f32, cx: f32, cy: f32) -> Self {
        Transform::translate(-cx, -cy)
            .then(&Transform::rotate(radians))
            .then(&Transform::translate(cx, cy))
    }

    /// `self` followed by `other` (i.e. `other × self` in column-vector convention).
    pub fn then(&self, other: &Transform) -> Transform {
        Transform {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    pub fn apply(&self, p: Point) -> Point {
        Point::new(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }

    /// True when the transform is a pure translation — lets renderers take fast paths.
    pub fn is_translation(&self) -> bool {
        self.a == 1.0 && self.b == 0.0 && self.c == 0.0 && self.d == 1.0
    }

    /// Approximate uniform scale factor, used to pick raster resolutions and to widen
    /// hairlines. Uses the geometric mean of the two axis scales.
    pub fn approx_scale(&self) -> f32 {
        let sx = (self.a * self.a + self.b * self.b).sqrt();
        let sy = (self.c * self.c + self.d * self.d).sqrt();
        ((sx * sy).abs()).sqrt()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathVerb {
    MoveTo,
    LineTo,
    QuadTo,
    CubicTo,
    Close,
}

/// The parameters in (0, 1) at which a cubic reaches a local extreme on one axis.
fn cubic_extrema(p0: f32, p1: f32, p2: f32, p3: f32) -> impl Iterator<Item = f32> {
    // B'(t)/3 = t^2 (A - 2B + C) + t (2B - 2A) + A, with A = p1-p0 etc.
    let (a1, b1, c1) = (p1 - p0, p2 - p1, p3 - p2);
    let a = a1 - 2.0 * b1 + c1;
    let b = 2.0 * (b1 - a1);
    let c = a1;
    let mut roots = [f32::NAN; 2];
    if a.abs() < 1e-6 {
        if b.abs() > 1e-6 {
            roots[0] = -c / b;
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let sq = disc.sqrt();
            roots[0] = (-b + sq) / (2.0 * a);
            roots[1] = (-b - sq) / (2.0 * a);
        }
    }
    roots.into_iter().filter(|t| *t > 0.0 && *t < 1.0)
}

/// A cubic evaluated on one axis.
fn cubic_at(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
}

/// A path as a verb stream plus a flat point buffer — compact, cheap to clone, and
/// directly replayable by both Canvas2D and a GPU tessellator.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    pub verbs: Vec<PathVerb>,
    pub points: Vec<Point>,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.verbs.is_empty()
    }

    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.verbs.push(PathVerb::MoveTo);
        self.points.push(Point::new(x, y));
        self
    }

    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.verbs.push(PathVerb::LineTo);
        self.points.push(Point::new(x, y));
        self
    }

    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) -> &mut Self {
        self.verbs.push(PathVerb::QuadTo);
        self.points.push(Point::new(cx, cy));
        self.points.push(Point::new(x, y));
        self
    }

    pub fn cubic_to(
        &mut self,
        c1x: f32,
        c1y: f32,
        c2x: f32,
        c2y: f32,
        x: f32,
        y: f32,
    ) -> &mut Self {
        self.verbs.push(PathVerb::CubicTo);
        self.points.push(Point::new(c1x, c1y));
        self.points.push(Point::new(c2x, c2y));
        self.points.push(Point::new(x, y));
        self
    }

    pub fn close(&mut self) -> &mut Self {
        self.verbs.push(PathVerb::Close);
        self
    }

    pub fn rect(r: Rect) -> Self {
        let mut p = Path::new();
        p.move_to(r.x, r.y)
            .line_to(r.right(), r.y)
            .line_to(r.right(), r.bottom())
            .line_to(r.x, r.bottom())
            .close();
        p
    }

    /// An ellipse inscribed in `r`, as four cubic segments.
    pub fn ellipse(r: Rect) -> Self {
        const K: f32 = 0.552_284_7; // 4/3·(√2−1): circle-to-cubic magic number
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (rx, ry) = (r.w / 2.0, r.h / 2.0);
        let (ox, oy) = (rx * K, ry * K);
        let mut p = Path::new();
        p.move_to(cx - rx, cy)
            .cubic_to(cx - rx, cy - oy, cx - ox, cy - ry, cx, cy - ry)
            .cubic_to(cx + ox, cy - ry, cx + rx, cy - oy, cx + rx, cy)
            .cubic_to(cx + rx, cy + oy, cx + ox, cy + ry, cx, cy + ry)
            .cubic_to(cx - ox, cy + ry, cx - rx, cy + oy, cx - rx, cy)
            .close();
        p
    }

    /// Appends an elliptical arc as cubic segments, starting from the current point.
    /// Angles are in radians, `sweep` may be negative. Matches DrawingML `arcTo`.
    pub fn arc_to(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32) {
        // Split into segments of at most 90° so the cubic approximation stays tight.
        let segments = ((sweep.abs() / (PI / 2.0)).ceil() as usize).max(1);
        let delta = sweep / segments as f32;
        let k = (4.0 / 3.0) * (delta / 4.0).tan();
        let mut theta = start;
        for _ in 0..segments {
            let (s0, c0) = theta.sin_cos();
            let (s1, c1) = (theta + delta).sin_cos();
            let p0 = Point::new(cx + rx * c0, cy + ry * s0);
            let p3 = Point::new(cx + rx * c1, cy + ry * s1);
            let c1p = Point::new(p0.x - k * rx * s0, p0.y + k * ry * c0);
            let c2p = Point::new(p3.x + k * rx * s1, p3.y - k * ry * c1);
            if self.verbs.is_empty() {
                self.move_to(p0.x, p0.y);
            }
            self.cubic_to(c1p.x, c1p.y, c2p.x, c2p.y, p3.x, p3.y);
            theta += delta;
        }
    }

    /// Control-point bounds. A cheap over-estimate — good enough for culling and for
    /// sizing gradient boxes, never used for hit-testing.
    /// The tight bounding box of the path.
    ///
    /// Curves are measured at their actual extrema rather than by the hull of their
    /// control points. The hull is easier and always at least as large, but a Bezier
    /// almost never reaches its control points: a quarter-circle's are about 5% outside
    /// the arc, so a shape drawn exactly inside its box reports a box 5% too big. That
    /// matters twice over — it makes the preset coverage test reject correct geometry,
    /// and it makes the renderer's culling keep work it could have skipped.
    pub fn bounds(&self) -> Rect {
        let Some(first) = self.points.first() else {
            return Rect::default();
        };
        let (mut minx, mut miny) = (f32::INFINITY, f32::INFINITY);
        let (mut maxx, mut maxy) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        let mut hit = |x: f32, y: f32| {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        };

        let mut i = 0usize;
        let mut cur = *first;
        for verb in &self.verbs {
            match verb {
                PathVerb::MoveTo | PathVerb::LineTo => {
                    if let Some(p) = self.points.get(i) {
                        hit(p.x, p.y);
                        cur = *p;
                    }
                    i += 1;
                }
                PathVerb::QuadTo => {
                    if let (Some(c), Some(p)) = (self.points.get(i), self.points.get(i + 1)) {
                        hit(p.x, p.y);
                        // Raise to a cubic and reuse the same extrema search.
                        let c1 = Point::new(
                            cur.x + 2.0 / 3.0 * (c.x - cur.x),
                            cur.y + 2.0 / 3.0 * (c.y - cur.y),
                        );
                        let c2 = Point::new(
                            p.x + 2.0 / 3.0 * (c.x - p.x),
                            p.y + 2.0 / 3.0 * (c.y - p.y),
                        );
                        for t in cubic_extrema(cur.x, c1.x, c2.x, p.x) {
                            hit(cubic_at(cur.x, c1.x, c2.x, p.x, t), cur.y);
                        }
                        for t in cubic_extrema(cur.y, c1.y, c2.y, p.y) {
                            hit(cur.x, cubic_at(cur.y, c1.y, c2.y, p.y, t));
                        }
                        cur = *p;
                    }
                    i += 2;
                }
                PathVerb::CubicTo => {
                    if let (Some(c1), Some(c2), Some(p)) = (
                        self.points.get(i),
                        self.points.get(i + 1),
                        self.points.get(i + 2),
                    ) {
                        hit(p.x, p.y);
                        for t in cubic_extrema(cur.x, c1.x, c2.x, p.x) {
                            hit(cubic_at(cur.x, c1.x, c2.x, p.x, t), cur.y);
                        }
                        for t in cubic_extrema(cur.y, c1.y, c2.y, p.y) {
                            hit(cur.x, cubic_at(cur.y, c1.y, c2.y, p.y, t));
                        }
                        cur = *p;
                    }
                    i += 3;
                }
                PathVerb::Close => {}
            }
        }
        if !minx.is_finite() {
            return Rect::default();
        }
        Rect::new(minx, miny, maxx - minx, maxy - miny)
    }

    /// Applies a transform in place. Used to bake group transforms into child paths when
    /// a renderer would rather not manage a transform stack.
    pub fn transform(&mut self, t: &Transform) {
        for p in &mut self.points {
            *p = t.apply(*p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_composition_matches_manual_application() {
        let t = Transform::translate(10.0, 5.0).then(&Transform::scale(2.0, 3.0));
        // translate first, then scale: (1,1) -> (11,6) -> (22,18)
        assert_eq!(t.apply(Point::new(1.0, 1.0)), Point::new(22.0, 18.0));
    }

    #[test]
    fn rotate_about_centre_leaves_centre_fixed() {
        let t = Transform::rotate_about(PI / 3.0, 100.0, 40.0);
        let p = t.apply(Point::new(100.0, 40.0));
        assert!((p.x - 100.0).abs() < 1e-3, "x drifted: {}", p.x);
        assert!((p.y - 40.0).abs() < 1e-3, "y drifted: {}", p.y);
    }

    #[test]
    fn quarter_turn_maps_x_axis_to_y_axis() {
        let t = Transform::rotate(PI / 2.0);
        let p = t.apply(Point::new(1.0, 0.0));
        assert!((p.x - 0.0).abs() < 1e-6);
        assert!((p.y - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ellipse_path_bounds_match_the_inscribing_rect() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        let b = Path::ellipse(r).bounds();
        // Control points of a circle-approximating cubic stay within the box.
        assert!((b.x - r.x).abs() < 0.01 && (b.y - r.y).abs() < 0.01);
        assert!((b.w - r.w).abs() < 0.01 && (b.h - r.h).abs() < 0.01);
    }

    #[test]
    fn arc_endpoints_land_on_the_ellipse() {
        let mut p = Path::new();
        p.arc_to(0.0, 0.0, 10.0, 10.0, 0.0, PI);
        let last = p.points.last().copied().unwrap_or_default();
        assert!((last.x + 10.0).abs() < 0.05, "arc end x = {}", last.x);
        assert!(last.y.abs() < 0.05, "arc end y = {}", last.y);
    }

    #[test]
    fn union_ignores_empty_rects() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(a.union(&Rect::default()), a);
        assert_eq!(Rect::default().union(&a), a);
    }
}
