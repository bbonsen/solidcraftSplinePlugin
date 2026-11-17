//! Spline profile geometry: pure 2D math, no host calls.
//!
//! Every profile is drawn in a sketch whose origin is the spline axis. A profile becomes one or two
//! cut *stages*:
//!
//! * tooth forms (involute, straight-sided, serration): first the turned diameter (external: a ring
//!   from the major diameter outward; internal: the minor-diameter bore), then one closed loop per
//!   tooth space;
//! * closed outlines (polygon, rounded polygon, lobed): one stage holding the outline (and, for an
//!   external profile, an outer circle so that the ring between them is cut).

use std::f64::consts::PI;

pub type P = [f64; 2];

/// One sketch curve, as the host draws it.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    /// A polyline through 2+ points (`sketch.line`).
    Line(Vec<P>),
    /// A circular arc through three points (`sketch.arc.three_point`).
    Arc { start: P, through: P, end: P },
    /// A fit-point spline (`sketch.spline.fit_point`).
    Spline { points: Vec<P>, periodic: bool },
    /// A full circle (`sketch.circle.center`).
    Circle { center: P, radius: f64 },
}

impl Curve {
    /// Every point the curve passes through, for checks and previews.
    pub fn points(&self) -> Vec<P> {
        match self {
            Curve::Line(p) => p.clone(),
            Curve::Arc { start, through, end } => vec![*start, *through, *end],
            Curve::Spline { points, .. } => points.clone(),
            Curve::Circle { center, radius } => vec![[center[0] + radius, center[1]]],
        }
    }
    pub fn first(&self) -> P {
        self.points()[0]
    }
    pub fn last(&self) -> P {
        match self {
            Curve::Spline { points, periodic: true } => points[0],
            Curve::Circle { .. } => self.first(),
            _ => *self.points().last().unwrap_or(&[0.0, 0.0]),
        }
    }
}

/// One sketch + extrude cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    /// Short label for the feature names ("Diameter", "Spaces", "Profile").
    pub label: &'static str,
    /// Every curve to draw.
    pub curves: Vec<Curve>,
    /// When the stage is `count` copies of one loop turned about the origin: the curves of one
    /// loop. The executor may draw the first loop and pattern it instead of drawing all of them.
    pub loop_len: Option<usize>,
    pub count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Involute,
    StraightSided,
    Serration,
    Polygon,
    RoundedPolygon,
    Lobed,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "involute" => Kind::Involute,
            "straight" | "straight_sided" => Kind::StraightSided,
            "serration" => Kind::Serration,
            "polygon" | "p_profile" => Kind::Polygon,
            "rounded_polygon" => Kind::RoundedPolygon,
            "lobed" | "hexalobular" => Kind::Lobed,
            _ => return None,
        })
    }
    pub fn id(self) -> &'static str {
        match self {
            Kind::Involute => "involute",
            Kind::StraightSided => "straight",
            Kind::Serration => "serration",
            Kind::Polygon => "polygon",
            Kind::RoundedPolygon => "rounded_polygon",
            Kind::Lobed => "lobed",
        }
    }
}

/// What the user asked for. Lengths in mm, angles in degrees.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub kind: Kind,
    pub internal: bool,
    /// Teeth, splines, sides or lobes.
    pub count: u32,
    pub module: f64,
    pub pressure_angle: f64,
    pub profile_shift: f64,
    /// Overrides; `None` (or 0 in the dialog) takes the standard value where there is one.
    pub major: Option<f64>,
    pub minor: Option<f64>,
    /// Straight-sided spline width B.
    pub width: f64,
    /// Serration: included angle between the two flanks of a tooth.
    pub flank_angle: f64,
    /// Rounded polygon corner radius.
    pub corner_radius: f64,
    /// Lobed profile tip radius; `None` picks one.
    pub lobe_radius: Option<f64>,
    /// Fit clearance, mm. Tooth forms: total side clearance at the pitch circle (external teeth get
    /// thinner by half of it, internal spaces wider by half). Outlines: on the diameter.
    pub clearance: f64,
    /// Turns the profile about the axis, degrees.
    pub rotation: f64,
    /// Fit points per involute flank.
    pub samples: usize,
}

impl Default for Spec {
    fn default() -> Self {
        Spec {
            kind: Kind::Involute,
            internal: false,
            count: 20,
            module: 1.0,
            pressure_angle: 30.0,
            profile_shift: 0.0,
            major: None,
            minor: None,
            width: 6.0,
            flank_angle: 90.0,
            corner_radius: 1.0,
            lobe_radius: None,
            clearance: 0.0,
            rotation: 0.0,
            samples: 9,
        }
    }
}

/// The resolved sizes, reported back to the user.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sizes {
    pub major: f64,
    pub minor: f64,
    pub pitch: Option<f64>,
    pub base: Option<f64>,
    /// Arc tooth thickness (external) or space width (internal) at the pitch circle.
    pub thickness: Option<f64>,
    pub notes: Vec<String>,
}

/// A planned spline: its sizes and its cut stages.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub sizes: Sizes,
    pub stages: Vec<Stage>,
}

fn polar(r: f64, a: f64) -> P {
    [r * a.cos(), r * a.sin()]
}
fn rot(p: P, a: f64) -> P {
    let (s, c) = a.sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
}
fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn scale(a: P, k: f64) -> P {
    [a[0] * k, a[1] * k]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn norm(a: P) -> f64 {
    dot(a, a).sqrt()
}
fn inv(a: f64) -> f64 {
    a.tan() - a
}
fn err<T>(msg: impl Into<String>) -> Result<T, String> {
    Err(msg.into())
}
fn positive(name: &str, v: f64) -> Result<f64, String> {
    if v.is_finite() && v > 0.0 { Ok(v) } else { err(format!("{name} must be a positive number")) }
}

/// The flank of the tooth (or internal space) centred on angle 0, upper side (positive angles).
#[derive(Clone, Copy, Debug)]
enum Flank {
    /// Involute from base radius `rb`; half-angle `k - inv(acos(rb/r))`, radial below `rb`.
    Involute { rb: f64, k: f64 },
    /// A straight line through `p` with direction `d` (pointing outward).
    Line { p: P, d: P },
}

impl Flank {
    /// The flank's polar angle at radius `r`, or `None` where it does not reach `r`.
    fn angle(self, r: f64) -> Option<f64> {
        match self {
            Flank::Involute { rb, k } => Some(if r <= rb { k } else { k - inv((rb / r).acos()) }),
            Flank::Line { p, d } => {
                let pd = dot(p, d);
                let disc = pd * pd - dot(p, p) + r * r;
                if disc < 0.0 {
                    return None;
                }
                let q = add(p, scale(d, -pd + disc.sqrt()));
                Some(q[1].atan2(q[0]))
            }
        }
    }
}

/// A cut region bounded by two mirrored flanks about the ray at angle `centre`.
struct Space {
    flank: Flank,
    /// External gaps sit between teeth (`h = pi/z - psi`); internal spaces are the tooth shape.
    gap: bool,
    z: u32,
    centre: f64,
}

impl Space {
    fn half(&self, r: f64) -> Option<f64> {
        let psi = self.flank.angle(r)?;
        Some(if self.gap { PI / self.z as f64 - psi } else { psi })
    }
    fn right(&self, r: f64) -> Option<P> {
        Some(polar(r, self.centre - self.half(r)?))
    }
    fn left(&self, r: f64) -> Option<P> {
        Some(polar(r, self.centre + self.half(r)?))
    }
}

/// Finds where `f` changes sign between `a` (where it is `fa_pos`) and `b`.
fn bisect(mut a: f64, mut b: f64, f: impl Fn(f64) -> f64) -> f64 {
    let fa = f(a) > 0.0;
    for _ in 0..80 {
        let m = 0.5 * (a + b);
        if (f(m) > 0.0) == fa { a = m } else { b = m }
    }
    0.5 * (a + b)
}

fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    let n = n.max(2);
    (0..n).map(|i| a + (b - a) * i as f64 / (n - 1) as f64).collect()
}

/// The closed loop of one space, from `r_in` to `r_out`.
fn space_loop(s: &Space, r_in: f64, r_out: f64, samples: usize) -> Result<Vec<Curve>, String> {
    let tiny = 1e-9;
    let half = |r: f64| s.half(r).unwrap_or(-1.0);
    let max_half = PI / s.z as f64;
    // Narrow the range to where the space is open; it may close to a point at one end.
    let (mut a, mut b) = (r_in, r_out);
    let (ha, hb) = (half(a), half(b));
    if ha <= tiny && hb <= tiny {
        return err("the tooth spaces close up completely: check the diameters, width and angles");
    }
    if ha <= tiny {
        a = bisect(b, a, half);
    }
    if hb <= tiny {
        b = bisect(a, b, half);
    }
    for r in linspace(a, b, 64) {
        if half(r) >= max_half - tiny {
            return err(format!(
                "the {} come to a point or vanish at diameter {:.3} mm: reduce the major diameter, the clearance or the flank angle",
                if s.gap { "teeth" } else { "internal teeth" },
                2.0 * r
            ));
        }
    }
    let pt = |side: fn(&Space, f64) -> Option<P>, r: f64| side(s, r).ok_or_else(|| format!("no flank at radius {r:.3} mm"));
    // One flank from a to b, as curves.
    let flank = |side: fn(&Space, f64) -> Option<P>| -> Result<Vec<Curve>, String> {
        match s.flank {
            Flank::Line { .. } => Ok(vec![Curve::Line(vec![pt(side, a)?, pt(side, b)?])]),
            Flank::Involute { rb, .. } => {
                let mut out = Vec::new();
                let start = if a < rb - 1e-9 {
                    let top = rb.min(b);
                    out.push(Curve::Line(vec![pt(side, a)?, pt(side, top)?]));
                    top
                } else {
                    a
                };
                if b > start + 1e-9 {
                    // Denser near the base circle, where the involute bends most.
                    let n = samples.max(3);
                    let pts = (0..n)
                        .map(|i| {
                            let t = i as f64 / (n - 1) as f64;
                            pt(side, start + (b - start) * t.powf(1.5))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    out.push(Curve::Spline { points: pts, periodic: false });
                }
                Ok(out)
            }
        }
    };
    let mut curves = flank(Space::right)?;
    if half(b) > tiny {
        curves.push(Curve::Arc { start: pt(Space::right, b)?, through: polar(b, s.centre), end: pt(Space::left, b)? });
    }
    let mut left = flank(Space::left)?;
    left.reverse();
    for c in left {
        curves.push(match c {
            Curve::Line(mut p) => {
                p.reverse();
                Curve::Line(p)
            }
            Curve::Spline { mut points, periodic } => {
                points.reverse();
                Curve::Spline { points, periodic }
            }
            other => other,
        });
    }
    if half(a) > tiny {
        curves.push(Curve::Arc { start: pt(Space::left, a)?, through: polar(a, s.centre), end: pt(Space::right, a)? });
    }
    // An apex is one point shared by both flanks: make it exactly equal.
    if half(a) <= tiny {
        let apex = curves[0].first();
        if let Some(Curve::Line(p) | Curve::Spline { points: p, .. }) = curves.last_mut() {
            if let Some(l) = p.last_mut() {
                *l = apex;
            }
        }
    }
    if half(b) <= tiny {
        // The right flank's end and the left flank's start meet at the outer apex.
        let n_right = flank(Space::right)?.len();
        let apex = curves[n_right - 1].last();
        if let Some(Curve::Line(p) | Curve::Spline { points: p, .. }) = curves.get_mut(n_right) {
            p[0] = apex;
        }
    }
    Ok(curves)
}

fn turn(curves: &[Curve], a: f64) -> Vec<Curve> {
    curves
        .iter()
        .map(|c| match c {
            Curve::Line(p) => Curve::Line(p.iter().map(|q| rot(*q, a)).collect()),
            Curve::Arc { start, through, end } => Curve::Arc { start: rot(*start, a), through: rot(*through, a), end: rot(*end, a) },
            Curve::Spline { points, periodic } => Curve::Spline { points: points.iter().map(|q| rot(*q, a)).collect(), periodic: *periodic },
            Curve::Circle { center, radius } => Curve::Circle { center: rot(*center, a), radius: *radius },
        })
        .collect()
}

fn circle(radius: f64) -> Curve {
    Curve::Circle { center: [0.0, 0.0], radius }
}

/// Standard diameter factors (multiples of the module, added to `z`) for ISO 4156-style flat-root
/// splines: (external major, external minor, internal major, internal minor).
fn involute_factors(alpha_deg: f64) -> (f64, f64, f64, f64) {
    let t30 = (1.0, -1.5, 1.5, -1.0);
    let t375 = (0.9, -1.4, 1.4, -0.9);
    let t45 = (0.8, -1.2, 1.2, -0.8);
    let lerp = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64), t: f64| {
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t, a.3 + (b.3 - a.3) * t)
    };
    if alpha_deg <= 30.0 {
        t30
    } else if alpha_deg <= 37.5 {
        lerp(t30, t375, (alpha_deg - 30.0) / 7.5)
    } else if alpha_deg <= 45.0 {
        lerp(t375, t45, (alpha_deg - 37.5) / 7.5)
    } else {
        t45
    }
}

/// Plans the cuts. `stock_radius` is the radius of the selected shaft or bore when known;
/// `outer_radius` is a radius beyond all material near the axis (for external rings).
pub fn plan(spec: &Spec, stock_radius: Option<f64>, outer_radius: Option<f64>) -> Result<Plan, String> {
    let z = spec.count;
    let rot0 = spec.rotation.to_radians();
    if !spec.clearance.is_finite() || spec.clearance < 0.0 {
        return err("clearance must be zero or positive");
    }
    let c = spec.clearance;
    let ext = !spec.internal;
    let mut sizes = Sizes::default();
    let margin = |major: f64| (0.1 * major / 2.0).max(1.0);
    let far = |major: f64| outer_radius.unwrap_or(0.0).max(stock_radius.unwrap_or(0.0)).max(major / 2.0) + margin(major);

    match spec.kind {
        Kind::Involute | Kind::StraightSided | Kind::Serration => {
            let (flank, major, minor) = match spec.kind {
                Kind::Involute => {
                    if !(6..=200).contains(&z) {
                        return err("an involute spline needs 6 to 200 teeth");
                    }
                    let m = positive("module", spec.module)?;
                    if !(14.5..=50.0).contains(&spec.pressure_angle) {
                        return err("pressure angle must be between 14.5 and 50 degrees");
                    }
                    let alpha = spec.pressure_angle.to_radians();
                    let x = spec.profile_shift;
                    if !x.is_finite() || x.abs() > 1.0 {
                        return err("profile shift coefficient must be between -1 and 1");
                    }
                    let zf = z as f64;
                    let r = m * zf / 2.0;
                    let rb = r * alpha.cos();
                    let f = involute_factors(spec.pressure_angle);
                    let shift = 2.0 * x * m;
                    let (major, minor) = if ext {
                        (spec.major.unwrap_or(m * (zf + f.0) + shift), spec.minor.unwrap_or(m * (zf + f.1) + shift))
                    } else {
                        (spec.major.unwrap_or(m * (zf + f.2) + shift), spec.minor.unwrap_or(m * (zf + f.3) + shift))
                    };
                    let basic = PI * m / 2.0 + 2.0 * x * m * alpha.tan();
                    let s = if ext { basic - c / 2.0 } else { basic + c / 2.0 };
                    positive("tooth thickness after clearance", s)?;
                    sizes.pitch = Some(2.0 * r);
                    sizes.base = Some(2.0 * rb);
                    sizes.thickness = Some(s);
                    if minor < 2.0 * rb && !ext {
                        sizes.notes.push("internal minor diameter is inside the base circle: the flank continues radially there".into());
                    }
                    (Flank::Involute { rb, k: s / (2.0 * r) + inv(alpha) }, major, minor)
                }
                Kind::StraightSided => {
                    if !(3..=48).contains(&z) {
                        return err("a straight-sided spline needs 3 to 48 splines");
                    }
                    let major = spec.major.ok_or("straight-sided splines need the major diameter D")?;
                    let minor = spec.minor.ok_or("straight-sided splines need the minor diameter d")?;
                    let b = positive("spline width B", spec.width)?;
                    let w = if ext { (b - c) / 2.0 } else { (b + c) / 2.0 };
                    positive("spline width after clearance", w)?;
                    let major = if ext { major } else { major + c };
                    sizes.thickness = Some(2.0 * w);
                    (Flank::Line { p: [0.0, w], d: [1.0, 0.0] }, major, minor)
                }
                _ => {
                    if !(6..=200).contains(&z) {
                        return err("a serration needs 6 to 200 teeth");
                    }
                    let major = spec.major.ok_or("serrations need the major diameter")?;
                    let minor = spec.minor.ok_or("serrations need the minor diameter")?;
                    if !(20.0..=150.0).contains(&spec.flank_angle) {
                        return err("serration flank angle must be between 20 and 150 degrees");
                    }
                    let (major, minor) = if ext { (major, minor) } else { (major + c, minor + c) };
                    let dp = (major + minor) / 2.0;
                    let s = PI * dp / (2.0 * z as f64);
                    let s = if ext { s - c / 2.0 } else { s + c / 2.0 };
                    positive("tooth thickness after clearance", s)?;
                    let psi = s / dp;
                    let half = (spec.flank_angle / 2.0).to_radians();
                    sizes.pitch = Some(dp);
                    sizes.thickness = Some(s);
                    (Flank::Line { p: polar(dp / 2.0, psi), d: [half.cos(), -half.sin()] }, major, minor)
                }
            };
            positive("major diameter", major)?;
            positive("minor diameter", minor)?;
            if minor >= major {
                return err(format!("the minor diameter ({minor:.3}) must be smaller than the major diameter ({major:.3})"));
            }
            sizes.major = major;
            sizes.minor = minor;
            // How far a space reaches past the turned diameter, so the two cuts overlap; small, and
            // smaller still where the teeth are nearly pointed there.
            let mut eps = ((major - minor) / 2.0 * 0.04).max(0.005);
            if ext {
                while eps > 1e-4 && flank.angle(major / 2.0 + eps).is_none_or(|a| a <= 1e-4) {
                    eps /= 2.0;
                }
            }
            let mut stages = Vec::new();
            let (space, r_in, r_out) = if ext {
                let r_far = far(major);
                if stock_radius.is_none_or(|r| r > major / 2.0 + 1e-6) {
                    stages.push(Stage { label: "Diameter", curves: vec![circle(major / 2.0), circle(r_far)], loop_len: None, count: 1 });
                }
                (Space { flank, gap: true, z, centre: PI / z as f64 }, minor / 2.0, major / 2.0 + eps)
            } else {
                if stock_radius.is_none_or(|r| r < minor / 2.0 - 1e-6) {
                    stages.push(Stage { label: "Bore", curves: vec![circle(minor / 2.0)], loop_len: None, count: 1 });
                }
                (Space { flank, gap: false, z, centre: 0.0 }, minor / 2.0 - eps, major / 2.0)
            };
            if r_in <= 0.0 {
                return err("the minor diameter is too small");
            }
            if let Flank::Line { p: [0.0, w], .. } = flank
                && r_in <= w
            {
                return err("the spline width is too large for the minor diameter");
            }
            let one = space_loop(&space, r_in, r_out, spec.samples)?;
            let step = 2.0 * PI / z as f64;
            let mut curves = Vec::new();
            for k in 0..z {
                curves.extend(turn(&one, rot0 + step * k as f64));
            }
            stages.push(Stage { label: "Spaces", loop_len: Some(one.len()), count: z as usize, curves });
            Ok(Plan { sizes, stages })
        }
        Kind::Polygon => {
            if !(2..=12).contains(&z) {
                return err("a polygon profile needs 2 to 12 sides");
            }
            let major = spec.major.ok_or("polygon profiles need the major (largest) diameter")?;
            let minor = spec.minor.ok_or("polygon profiles need the minor (smallest) diameter")?;
            if minor >= major {
                return err("the minor diameter must be smaller than the major diameter");
            }
            let n = z as f64;
            let delta = if ext { -c / 2.0 } else { c / 2.0 };
            let r = (major + minor) / 4.0 + delta;
            let e = (major - minor) / 4.0;
            if e > r / (n * n - 1.0) {
                return err(format!(
                    "this polygon would not be convex: for {z} sides the eccentricity (major - minor)/4 must be at most {:.3} mm",
                    r / (n * n - 1.0)
                ));
            }
            sizes.major = 2.0 * (r + e);
            sizes.minor = 2.0 * (r - e);
            sizes.notes.push(format!("eccentricity e = {e:.4} mm, mean diameter = {:.4} mm", 2.0 * r));
            let count = (16 * z as usize).clamp(48, 480);
            let pts = (0..count)
                .map(|i| {
                    let t = 2.0 * PI * i as f64 / count as f64;
                    let p = r - e * (n * t).cos();
                    let dp = n * e * (n * t).sin();
                    rot([p * t.cos() - dp * t.sin(), p * t.sin() + dp * t.cos()], rot0)
                })
                .collect();
            let outline = Curve::Spline { points: pts, periodic: true };
            Ok(Plan { stages: vec![outline_stage(ext, outline_vec(outline), far(sizes.major))], sizes })
        }
        Kind::RoundedPolygon => {
            if !(3..=12).contains(&z) {
                return err("a rounded polygon needs 3 to 12 sides");
            }
            let n = z as f64;
            let flats = match (spec.minor, spec.major) {
                (Some(s), _) => s,
                (None, Some(d)) => d * (PI / n).cos(),
                (None, None) => return err("a rounded polygon needs its across-flats (minor) or across-corners (major) size"),
            };
            positive("across-flats size", flats)?;
            let delta = if ext { -c / 2.0 } else { c / 2.0 };
            let a = flats / 2.0 + delta;
            let rc = (spec.corner_radius + delta).max(0.0);
            if !spec.corner_radius.is_finite() || spec.corner_radius < 0.0 {
                return err("corner radius must be zero or positive");
            }
            if rc >= a - 1e-6 {
                return err(format!("the corner radius must be smaller than half the across-flats size ({a:.3} mm)"));
            }
            let a0 = a - rc;
            let rv = a0 / (PI / n).cos();
            sizes.minor = 2.0 * a;
            sizes.major = 2.0 * (rv + rc);
            let normal = |k: usize| rot0 + 2.0 * PI * k as f64 / n;
            let mut curves = Vec::new();
            let zu = z as usize;
            // Tangent points: on side k, the corner before it ends at `start[k]`, the one after begins at `end[k]`.
            let corner = |k: usize| polar(rv, normal(k) + PI / n);
            for k in 0..zu {
                let kn = (k + 1) % zu;
                let c0 = corner(k);
                let side_end = add(c0, polar(rc, normal(k)));
                let prev = corner((k + zu - 1) % zu);
                let side_start = add(prev, polar(rc, normal(k)));
                curves.push(Curve::Line(vec![side_start, side_end]));
                if rc > 1e-9 {
                    let next_start = add(c0, polar(rc, normal(kn)));
                    curves.push(Curve::Arc { start: side_end, through: add(c0, polar(rc, normal(k) + PI / n)), end: next_start });
                }
            }
            Ok(Plan { stages: vec![outline_stage(ext, curves, far(sizes.major))], sizes })
        }
        Kind::Lobed => {
            if !(3..=12).contains(&z) {
                return err("a lobed profile needs 3 to 12 lobes");
            }
            let major = spec.major.ok_or("lobed profiles need the major (lobe tip) diameter")?;
            let minor = spec.minor.ok_or("lobed profiles need the minor (valley) diameter")?;
            if minor >= major {
                return err("the minor diameter must be smaller than the major diameter");
            }
            let n = z as f64;
            let th = PI / n;
            let (ra, rbv) = (major / 2.0, minor / 2.0);
            let r1 = spec.lobe_radius.unwrap_or_else(|| default_lobe_radius(ra, rbv, th));
            positive("lobe radius", r1)?;
            let c1 = ra - r1;
            if c1 <= 0.0 {
                return err("the lobe radius must be smaller than half the major diameter");
            }
            // Valley arc radius r2 with centre at rbv + r2, tangent to the neighbouring lobes.
            let den = 2.0 * r1 - 2.0 * rbv + 2.0 * c1 * th.cos();
            let num = c1 * c1 + rbv * rbv - 2.0 * c1 * rbv * th.cos() - r1 * r1;
            let r2 = num / den;
            if !(r2.is_finite() && r2 > 0.0) || den <= 0.0 {
                return err("these lobe sizes have no tangent valley arc: try a smaller lobe radius or a larger minor diameter");
            }
            let c2 = rbv + r2;
            let delta = if ext { -c / 2.0 } else { c / 2.0 };
            let (q1, q2) = (r1 + delta, r2 - delta);
            if q1 <= 0.0 || q2 <= 0.0 {
                return err("the clearance is too large for these lobe radii");
            }
            sizes.major = 2.0 * (c1 + q1);
            sizes.minor = 2.0 * (c2 - q2);
            sizes.notes.push(format!("lobe radius {r1:.4} mm, valley radius {r2:.4} mm"));
            let lobe_c = |k: usize| polar(c1, rot0 + 2.0 * th * k as f64);
            let valley_c = |k: usize| polar(c2, rot0 + 2.0 * th * k as f64 + th);
            let touch = |l: P, v: P| {
                let d = sub(v, l);
                add(l, scale(d, q1 / norm(d)))
            };
            let zu = z as usize;
            let mut curves = Vec::new();
            for k in 0..zu {
                let (l, v, ln) = (lobe_c(k), valley_c(k), lobe_c((k + 1) % zu));
                let vp = valley_c((k + zu - 1) % zu);
                let a_start = touch(l, vp);
                let a_end = touch(l, v);
                let tip = add(l, polar(q1, rot0 + 2.0 * th * k as f64));
                curves.push(Curve::Arc { start: a_start, through: tip, end: a_end });
                let next_start = touch(ln, v);
                let bottom = add(v, polar(-q2, rot0 + 2.0 * th * k as f64 + th));
                curves.push(Curve::Arc { start: a_end, through: bottom, end: next_start });
            }
            Ok(Plan { stages: vec![outline_stage(ext, curves, far(sizes.major))], sizes })
        }
    }
}

/// A lobe radius that leaves a valley radius of about the same size.
fn default_lobe_radius(ra: f64, rb: f64, th: f64) -> f64 {
    // Solve r2(r1) = r1 by bisection over the valid range.
    let r2 = |r1: f64| {
        let c1 = ra - r1;
        let den = 2.0 * r1 - 2.0 * rb + 2.0 * c1 * th.cos();
        (c1 * c1 + rb * rb - 2.0 * c1 * rb * th.cos() - r1 * r1) / den
    };
    let (mut lo, mut hi) = (1e-6 * ra, ra * 0.999);
    for _ in 0..80 {
        let m = 0.5 * (lo + hi);
        let v = r2(m);
        if !v.is_finite() || v < 0.0 || v < m { hi = m } else { lo = m }
    }
    0.5 * (lo + hi)
}

fn outline_vec(c: Curve) -> Vec<Curve> {
    vec![c]
}

fn outline_stage(ext: bool, mut outline: Vec<Curve>, far: f64) -> Stage {
    if ext {
        outline.insert(0, circle(far));
    }
    Stage { label: "Profile", curves: outline, loop_len: None, count: 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P, b: P) -> bool {
        norm(sub(a, b)) < 1e-9
    }

    /// Every stage's non-circle curves form closed loops with exactly shared end points.
    fn assert_closed(stage: &Stage) {
        let open: Vec<&Curve> = stage.curves.iter().filter(|c| !matches!(c, Curve::Circle { .. } | Curve::Spline { periodic: true, .. })).collect();
        if open.is_empty() {
            return;
        }
        let per = stage.loop_len.unwrap_or(open.len());
        for chunk in open.chunks(per) {
            for (i, c) in chunk.iter().enumerate() {
                let next = chunk[(i + 1) % chunk.len()];
                assert_eq!(c.last(), next.first(), "loop not closed exactly at curve {i} of {stage:?}");
            }
        }
    }

    fn spec(kind: Kind, internal: bool) -> Spec {
        let mut s = Spec { kind, internal, ..Spec::default() };
        match kind {
            Kind::Involute => {}
            Kind::StraightSided => {
                s.count = 6;
                s.minor = Some(23.0);
                s.major = Some(26.0);
                s.width = 6.0;
            }
            Kind::Serration => {
                s.count = 28;
                s.major = Some(20.0);
                s.minor = Some(18.4);
                s.flank_angle = 90.0;
            }
            Kind::Polygon => {
                s.count = 3;
                s.major = Some(22.0);
                s.minor = Some(18.0);
            }
            Kind::RoundedPolygon => {
                s.count = 6;
                s.minor = Some(17.0);
                s.corner_radius = 1.5;
            }
            Kind::Lobed => {
                s.count = 6;
                s.major = Some(10.0);
                s.minor = Some(7.2);
            }
        }
        s
    }

    #[test]
    fn every_kind_plans_closed_loops_both_ways() {
        for kind in [Kind::Involute, Kind::StraightSided, Kind::Serration, Kind::Polygon, Kind::RoundedPolygon, Kind::Lobed] {
            for internal in [false, true] {
                let plan = plan(&spec(kind, internal), None, None).unwrap_or_else(|e| panic!("{kind:?} internal={internal}: {e}"));
                assert!(!plan.stages.is_empty());
                for st in &plan.stages {
                    assert_closed(st);
                }
            }
        }
    }

    #[test]
    fn involute_iso_sizes_and_thickness() {
        let p = plan(&spec(Kind::Involute, false), None, None).unwrap();
        // m=1, z=20, 30 deg: Dee = 21, Die = 18.5, D = 20, Db = 20 cos 30.
        assert!((p.sizes.major - 21.0).abs() < 1e-12);
        assert!((p.sizes.minor - 18.5).abs() < 1e-12);
        assert!((p.sizes.base.unwrap() - 20.0 * (30f64).to_radians().cos()).abs() < 1e-12);
        let spaces = p.stages.iter().find(|s| s.label == "Spaces").unwrap();
        assert_eq!(spaces.count, 20);
        // At the pitch radius the gap spans pi*m/2 of arc: its flank points sit at angle psi = pi/(2z).
        let one = &spaces.curves[..spaces.loop_len.unwrap()];
        let space = Space { flank: Flank::Involute { rb: 10.0 * (30f64).to_radians().cos(), k: PI / 40.0 + inv((30f64).to_radians()) }, gap: true, z: 20, centre: PI / 20.0 };
        let at_pitch = space.right(10.0).unwrap();
        assert!((at_pitch[1].atan2(at_pitch[0]) - PI / 40.0).abs() < 1e-12);
        // Every spline fit point lies on the involute: r and polar angle agree.
        for c in one {
            if let Curve::Spline { points, .. } = c {
                for q in points {
                    let r = norm(*q);
                    let ang = q[1].atan2(q[0]);
                    let expect = space.right(r).unwrap();
                    let left = space.left(r).unwrap();
                    let e1 = (expect[1].atan2(expect[0]) - ang).abs();
                    let e2 = (left[1].atan2(left[0]) - ang).abs();
                    assert!(e1 < 1e-9 || e2 < 1e-9);
                }
            }
        }
    }

    #[test]
    fn internal_and_external_fit_with_clearance() {
        let mut e = spec(Kind::Involute, false);
        e.clearance = 0.1;
        let mut i = e.clone();
        i.internal = true;
        let pe = plan(&e, None, None).unwrap();
        let pi = plan(&i, None, None).unwrap();
        assert!((pi.sizes.thickness.unwrap() - pe.sizes.thickness.unwrap() - 0.1).abs() < 1e-12);
        assert!(pi.sizes.major > pe.sizes.major && pi.sizes.minor > pe.sizes.minor);
        assert!(pi.sizes.minor < pe.sizes.major);
    }

    #[test]
    fn straight_sided_flanks_are_parallel_to_the_tooth() {
        let p = plan(&spec(Kind::StraightSided, false), None, None).unwrap();
        let st = p.stages.last().unwrap();
        let Curve::Line(l) = &st.curves[0] else { panic!("expected a line") };
        // Tooth 0 side is y = B/2 = 3.
        assert!((l[0][1] - 3.0).abs() < 1e-12 && (l[1][1] - 3.0).abs() < 1e-12);
        assert!((norm(l[0]) - 11.5).abs() < 1e-12);
    }

    #[test]
    fn known_stock_skips_turning() {
        let s = spec(Kind::Involute, false);
        assert_eq!(plan(&s, Some(10.5), None).unwrap().stages.len(), 1);
        assert_eq!(plan(&s, Some(12.0), None).unwrap().stages.len(), 2);
        let mut i = s.clone();
        i.internal = true;
        // Internal minor is 19 mm: an existing 19 mm bore needs no bore cut.
        assert_eq!(plan(&i, Some(9.5), None).unwrap().stages.len(), 1);
        assert_eq!(plan(&i, Some(5.0), None).unwrap().stages.len(), 2);
    }

    #[test]
    fn polygon_radii_and_convexity() {
        let p = plan(&spec(Kind::Polygon, false), None, None).unwrap();
        let Curve::Spline { points, periodic } = &p.stages[0].curves[1] else { panic!() };
        assert!(*periodic);
        let rmax = points.iter().map(|q| norm(*q)).fold(0.0, f64::max);
        let rmin = points.iter().map(|q| norm(*q)).fold(f64::MAX, f64::min);
        assert!((rmax - 11.0).abs() < 0.01 && (rmin - 9.0).abs() < 1e-9, "{rmin} {rmax}");
        let mut bad = spec(Kind::Polygon, false);
        bad.minor = Some(10.0);
        assert!(plan(&bad, None, None).is_err());
    }

    #[test]
    fn lobed_arcs_are_tangent_and_sized() {
        let p = plan(&spec(Kind::Lobed, true), None, None).unwrap();
        assert!((p.sizes.major - 10.0).abs() < 1e-9 && (p.sizes.minor - 7.2).abs() < 1e-9);
        assert_eq!(p.stages[0].curves.len(), 12);
    }

    #[test]
    fn rounded_hexagon_sizes() {
        let p = plan(&spec(Kind::RoundedPolygon, false), None, None).unwrap();
        assert!((p.sizes.minor - 17.0).abs() < 1e-12);
        let sharp = 17.0 / (PI / 6.0).cos();
        assert!(p.sizes.major < sharp && p.sizes.major > 17.0);
        assert_eq!(p.stages[0].curves.len(), 13);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let mut s = spec(Kind::Involute, false);
        s.module = -1.0;
        assert!(plan(&s, None, None).is_err());
        s.module = f64::NAN;
        assert!(plan(&s, None, None).is_err());
        let mut s = spec(Kind::Involute, false);
        s.major = Some(40.0);
        assert!(plan(&s, None, None).unwrap_err().contains("point"));
        let mut s = spec(Kind::StraightSided, false);
        s.width = 20.0;
        assert!(plan(&s, None, None).is_err());
        let mut s = spec(Kind::Serration, false);
        s.count = 0;
        assert!(plan(&s, None, None).is_err());
    }

    #[test]
    fn serration_with_wide_angle_closes_to_an_apex() {
        let mut s = spec(Kind::Serration, false);
        s.minor = Some(18.0);
        let p = plan(&s, None, None).unwrap();
        // No root arc: the space is a triangle-like loop of flank, outer arc, flank.
        assert_eq!(p.stages[1].loop_len, Some(3));
        for st in &p.stages {
            assert_closed(st);
        }
        let _ = close;
    }
}
