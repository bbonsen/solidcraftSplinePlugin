//! Renders every spline type, external and internal, as SVG: `cargo run --example preview -- out.svg`.
//! Cut regions are shaded; the dashed circles are the major and minor diameters.

use solidcraft_spline::geom::{Curve, Kind, P, Spec, plan};
use std::fmt::Write;

fn arc_points(a: P, m: P, b: P) -> Vec<P> {
    // Circle through three points.
    let d = 2.0 * (a[0] * (m[1] - b[1]) + m[0] * (b[1] - a[1]) + b[0] * (a[1] - m[1]));
    if d.abs() < 1e-12 {
        return vec![a, b];
    }
    let sq = |p: P| p[0] * p[0] + p[1] * p[1];
    let ux = (sq(a) * (m[1] - b[1]) + sq(m) * (b[1] - a[1]) + sq(b) * (a[1] - m[1])) / d;
    let uy = (sq(a) * (b[0] - m[0]) + sq(m) * (a[0] - b[0]) + sq(b) * (m[0] - a[0])) / d;
    let r = ((a[0] - ux).powi(2) + (a[1] - uy).powi(2)).sqrt();
    let ang = |p: P| (p[1] - uy).atan2(p[0] - ux);
    let (ta, tm, mut tb) = (ang(a), ang(m), ang(b));
    let tau = std::f64::consts::TAU;
    // Go from a to b the way that passes m.
    let norm = |x: f64| ((x % tau) + tau) % tau;
    let ccw = norm(tm - ta) < norm(tb - ta);
    tb = if ccw { ta + norm(tb - ta) } else { ta - norm(ta - tb) };
    (0..=24).map(|i| {
        let t = ta + (tb - ta) * i as f64 / 24.0;
        [ux + r * t.cos(), uy + r * t.sin()]
    }).collect()
}

fn path(curves: &[Curve], s: f64, cx: f64, cy: f64) -> String {
    let mut d = String::new();
    let mut first = true;
    let mut pt = |d: &mut String, p: P, first: &mut bool| {
        let _ = write!(d, "{}{:.3},{:.3} ", if *first { "M" } else { "L" }, cx + p[0] * s, cy - p[1] * s);
        *first = false;
    };
    let mut last: Option<P> = None;
    for c in curves {
        if last != Some(c.first()) {
            first = true;
        }
        last = Some(c.last());
        match c {
            Curve::Circle { center, radius } => {
                let _ = write!(d, "M{:.3},{:.3} m{:.3},0 a{r:.3},{r:.3} 0 1,0 {:.3},0 a{r:.3},{r:.3} 0 1,0 {:.3},0 Z ", cx + center[0] * s, cy - center[1] * s, -radius * s, 2.0 * radius * s, -2.0 * radius * s, r = radius * s);
                first = true;
            }
            Curve::Line(p) | Curve::Spline { points: p, .. } => {
                for q in p {
                    pt(&mut d, *q, &mut first);
                }
                if matches!(c, Curve::Spline { periodic: true, .. }) {
                    d.push_str("Z ");
                    first = true;
                }
            }
            Curve::Arc { start, through, end } => {
                for q in arc_points(*start, *through, *end) {
                    pt(&mut d, q, &mut first);
                }
            }
        }
    }
    d
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "preview.svg".into());
    let cases: Vec<(&str, Spec)> = vec![
        ("Involute 30° m1 z20", Spec { kind: Kind::Involute, count: 20, module: 1.0, ..Spec::default() }),
        ("Involute 45° m0.8 z30", Spec { kind: Kind::Involute, count: 30, module: 0.8, pressure_angle: 45.0, ..Spec::default() }),
        ("Straight 6×23×26 B6", Spec { kind: Kind::StraightSided, count: 6, minor: Some(23.0), major: Some(26.0), width: 6.0, ..Spec::default() }),
        ("Serration 90° z28", Spec { kind: Kind::Serration, count: 28, minor: Some(18.4), major: Some(20.0), ..Spec::default() }),
        ("Polygon P3 22/18", Spec { kind: Kind::Polygon, count: 3, minor: Some(18.0), major: Some(22.0), ..Spec::default() }),
        ("Polygon P4 21.6/19", Spec { kind: Kind::Polygon, count: 4, minor: Some(19.0), major: Some(21.6), ..Spec::default() }),
        ("Rounded hex AF17 r1.5", Spec { kind: Kind::RoundedPolygon, count: 6, minor: Some(17.0), corner_radius: 1.5, ..Spec::default() }),
        ("Lobed 6 Ø20/14.4", Spec { kind: Kind::Lobed, count: 6, minor: Some(14.4), major: Some(20.0), ..Spec::default() }),
    ];
    let (cell, s) = (260.0, 9.0);
    let mut svg = String::new();
    let w = cell * 2.0;
    let h = cell * cases.len() as f64;
    let _ = writeln!(svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="sans-serif" font-size="13"><rect width="100%" height="100%" fill="white"/>"#);
    for (row, (title, spec)) in cases.iter().enumerate() {
        for (col, internal) in [false, true].into_iter().enumerate() {
            let spec = Spec { internal, ..spec.clone() };
            let (cx, cy) = (cell * col as f64 + cell / 2.0, cell * row as f64 + cell / 2.0 + 10.0);
            let _ = writeln!(svg, r#"<text x="{}" y="{}">{} — {}</text>"#, cell * col as f64 + 8.0, cell * row as f64 + 16.0, title, if internal { "internal" } else { "external" });
            match plan(&spec, None, Some(13.0)) {
                Err(e) => {
                    let _ = writeln!(svg, r#"<text x="{}" y="{cy}" fill="red">{e}</text>"#, cell * col as f64 + 8.0);
                }
                Ok(p) => {
                    // Stock: a shaft of radius 13 (external), or a hub (internal).
                    let _ = writeln!(svg, r##"<circle cx="{cx}" cy="{cy}" r="{}" fill="#cfd8e3" stroke="#555"/>"##, if internal { 13.0 * s } else { 12.0 * s });
                    for st in &p.stages {
                        let _ = writeln!(svg, r#"<path d="{}" fill="white" fill-rule="evenodd" stroke="crimson" stroke-width="0.8"/>"#, path(&st.curves, s, cx, cy));
                    }
                    for d in [p.sizes.major, p.sizes.minor] {
                        let _ = writeln!(svg, r##"<circle cx="{cx}" cy="{cy}" r="{:.3}" fill="none" stroke="#2a6" stroke-dasharray="3 3" stroke-width="0.6"/>"##, d / 2.0 * s);
                    }
                }
            }
        }
    }
    svg.push_str("</svg>\n");
    std::fs::write(&out, svg).expect("write the SVG");
    println!("wrote {out}");
}
