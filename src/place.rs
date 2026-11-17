//! Where the spline goes: its axis, its start plane and which way the material lies.

use crate::{Failure, Host, Options, input};
use serde_json::{Value, json};

pub type V3 = [f64; 3];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn unit(a: V3) -> Option<V3> {
    let n = dot(a, a).sqrt();
    (n.is_finite() && n > 1e-12).then(|| scale(a, 1.0 / n))
}

/// Sketch x and y directions for a plane whose normal (x × y) is `axis`.
pub fn frame(axis: V3) -> (V3, V3) {
    // World X when the axis is near Z (so an XY sketch keeps its usual axes), else world Z.
    let refv = if axis[2].abs() >= 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
    let x = unit(sub(refv, scale(axis, dot(refv, axis)))).unwrap_or([1.0, 0.0, 0.0]);
    (x, cross(axis, x))
}

pub fn parse_v3(s: &str) -> Option<V3> {
    let v: Vec<f64> = s.split([',', ' ', ';']).filter(|t| !t.is_empty()).map(|t| t.parse::<f64>().ok().filter(|x| x.is_finite())).collect::<Option<_>>()?;
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

pub fn parse_axis(s: &str) -> Option<V3> {
    let t = s.trim().to_ascii_uppercase();
    let (sign, name) = match t.strip_prefix('-') {
        Some(r) => (-1.0, r.trim().to_string()),
        None => (1.0, t.trim_start_matches('+').trim().to_string()),
    };
    let base = match name.as_str() {
        "X" => Some([1.0, 0.0, 0.0]),
        "Y" => Some([0.0, 1.0, 0.0]),
        "Z" => Some([0.0, 0.0, 1.0]),
        _ => None,
    };
    match base {
        Some(b) => Some(scale(b, sign)),
        None => parse_v3(s).and_then(unit),
    }
}

/// The spline's placement.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// On the axis, in the start plane.
    pub center: V3,
    /// Unit axis pointing into the material.
    pub axis: V3,
    /// Radius of the selected shaft or bore, when the selection says.
    pub radius: Option<f64>,
    /// A radius beyond all of the body's material around the axis.
    pub outer_radius: Option<f64>,
    pub body: Option<String>,
}

fn v3(v: &Value, keys: &[&str]) -> Option<V3> {
    keys.iter().find_map(|k| {
        let a = v.get(*k)?.as_array()?;
        if a.len() != 3 {
            return None;
        }
        Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
    })
}
fn number(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| v.get(*k)?.as_f64()).filter(|x| x.is_finite())
}

/// What one selected item tells us.
#[derive(Clone, Debug, PartialEq)]
pub enum Picked {
    /// A circular edge (or arc): centre, axis, radius.
    Circle { body: Option<String>, center: V3, axis: V3, radius: f64 },
    /// A cylindrical face.
    Cylinder { body: Option<String>, origin: V3, axis: V3, radius: f64 },
    /// A flat face; its centre comes from `model.faces`.
    Plane { body: Option<String>, index: Option<u64>, normal: V3, point: Option<V3> },
    Other,
}

pub fn picked(item: &Value) -> Picked {
    let body = item.get("body").and_then(Value::as_str).map(String::from);
    let kind = item.get("kind").or_else(|| item.get("type")).and_then(Value::as_str).unwrap_or("");
    match kind {
        "edge" => {
            let radius = number(item, &["radius", "radius_mm"]).or_else(|| number(item, &["diameter", "diameter_mm"]).map(|d| d / 2.0));
            let center = v3(item, &["center", "centre", "center_mm"]);
            let axis = v3(item, &["axis", "normal"]).and_then(unit);
            match (center, axis, radius) {
                (Some(center), Some(axis), Some(radius)) if radius > 0.0 => Picked::Circle { body, center, axis, radius },
                _ => Picked::Other,
            }
        }
        "face" => {
            let surface = item.get("surface").cloned().unwrap_or(Value::Null);
            match surface.get("type").and_then(Value::as_str) {
                Some("cylinder") => match (v3(&surface, &["origin"]), v3(&surface, &["axis"]).and_then(unit), number(&surface, &["radius"])) {
                    (Some(origin), Some(axis), Some(radius)) => Picked::Cylinder { body, origin, axis, radius },
                    _ => Picked::Other,
                },
                Some("plane") => match v3(&surface, &["normal"]).and_then(unit) {
                    Some(normal) => Picked::Plane { body, index: item.get("index").and_then(Value::as_u64), normal, point: v3(&surface, &["point"]) },
                    None => Picked::Other,
                },
                _ => Picked::Other,
            }
        }
        _ => Picked::Other,
    }
}

const HOW: &str = "select a circular edge of the shaft end or bore, or a cylindrical face together with the flat face where the spline starts, or a flat end face (or fill in Center and Axis)";

/// Centre, axis and radius from the selection or the typed centre/axis, with the axis pointing
/// into the material.
pub fn locate<H: Host>(host: &mut H, opts: &Options, internal: bool) -> Result<Placement, Failure<H::Error>> {
    let (center, axis, radius, mut body, normal) = if let Some(center) = opts.center {
        (center, opts.axis.unwrap_or([0.0, 0.0, 1.0]), None, opts.body.clone(), None)
    } else {
        let sel = host.run("selection.get", &json!({})).map_err(Failure::Host)?;
        let items: Vec<Picked> = sel.get("items").and_then(Value::as_array).map(|a| a.iter().map(picked).collect()).unwrap_or_default();
        let circle = items.iter().find_map(|p| match p {
            Picked::Circle { body, center, axis, radius } => Some((body.clone(), *center, *axis, *radius)),
            _ => None,
        });
        let cyl = items.iter().find_map(|p| match p {
            Picked::Cylinder { body, origin, axis, radius } => Some((body.clone(), *origin, *axis, *radius)),
            _ => None,
        });
        let plane = items.iter().find_map(|p| match p {
            Picked::Plane { body, index, normal, point } => Some((body.clone(), *index, *normal, *point)),
            _ => None,
        });
        match (circle, cyl, plane) {
            (Some((b, c, a, r)), _, plane) => (c, a, Some(r), b, plane.map(|p| p.2)),
            (None, Some((b, o, a, r)), Some((pb, _, n, Some(pt)))) => {
                // Where the cylinder's axis meets the plane.
                let denom = dot(a, n);
                if denom.abs() < 1e-6 {
                    return Err(input("selection", "the flat face is parallel to the cylinder's axis"));
                }
                let t = dot(sub(pt, o), n) / denom;
                (add(o, scale(a, t)), a, Some(r), b.or(pb), Some(n))
            }
            (None, Some(_), _) => return Err(input("selection", format!("a cylindrical face alone does not say where the spline starts: {HOW}"))),
            (None, None, Some((b, index, n, _))) => {
                let b = b.ok_or_else(|| input("selection", "the selected face has no body"))?;
                let center = face_centroid(host, &b, index)?.ok_or_else(|| input("selection", format!("could not find the selected face's centre: {HOW}")))?;
                (center, n, None, Some(b), Some(n))
            }
            (None, None, None) => return Err(input("selection", format!("nothing usable is selected: {HOW}"))),
        }
    };
    if opts.body.is_some() {
        body = opts.body.clone();
    }
    let axis = unit(axis).ok_or_else(|| input("axis", "the axis has no direction"))?;
    let mut placement = Placement { center, axis, radius, outer_radius: None, body: body.clone() };
    if let Some(b) = &body
        && let Some((min, max)) = bbox(host, b)?
    {
        let mid = scale(add(min, max), 0.5);
        let size = dot(sub(max, min), sub(max, min)).sqrt();
        let along = dot(sub(mid, center), axis);
        if along.abs() > 1e-6 * size.max(1.0) {
            if along < 0.0 {
                placement.axis = scale(axis, -1.0);
            }
        } else if let Some(n) = normal {
            // A face normal points out of the material.
            placement.axis = scale(n, -1.0);
        }
        if !internal {
            let mut far: f64 = 0.0;
            for i in 0..8 {
                let corner = [if i & 1 == 0 { min[0] } else { max[0] }, if i & 2 == 0 { min[1] } else { max[1] }, if i & 4 == 0 { min[2] } else { max[2] }];
                let d = sub(corner, center);
                let radial = sub(d, scale(placement.axis, dot(d, placement.axis)));
                far = far.max(dot(radial, radial).sqrt());
            }
            placement.outer_radius = Some(far);
        }
    } else if let Some(n) = normal
        && dot(n, placement.axis) > 0.0
    {
        placement.axis = scale(placement.axis, -1.0);
    }
    Ok(placement)
}

fn face_centroid<H: Host>(host: &mut H, body: &str, index: Option<u64>) -> Result<Option<V3>, Failure<H::Error>> {
    let Some(index) = index else { return Ok(None) };
    let reply = host.run("model.faces", &json!({ "body": body })).map_err(Failure::Host)?;
    let faces = reply.get("faces").and_then(Value::as_array).or_else(|| reply.as_array());
    Ok(faces.and_then(|f| f.iter().find(|x| x.get("index").and_then(Value::as_u64) == Some(index))).and_then(|f| v3(f, &["centroid"])))
}

/// The body's bounding box from `inspect.measure`.
fn bbox<H: Host>(host: &mut H, body: &str) -> Result<Option<(V3, V3)>, Failure<H::Error>> {
    let reply = host.run("inspect.measure", &json!({ "bodies": [body] })).map_err(Failure::Host)?;
    fn find(v: &Value, body: &str) -> Option<(V3, V3)> {
        match v {
            Value::Object(m) => {
                if m.get("name").and_then(Value::as_str) == Some(body)
                    && let Some(b) = m.get("bbox")
                    && let (Some(min), Some(max)) = (v3(b, &["min"]), v3(b, &["max"]))
                {
                    return Some((min, max));
                }
                m.values().find_map(|x| find(x, body))
            }
            Value::Array(a) => a.iter().find_map(|x| find(x, body)),
            _ => None,
        }
    }
    Ok(find(&reply, body))
}
