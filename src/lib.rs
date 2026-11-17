//! SolidCraft plugin: cut involute, straight-sided, serrated, polygon (P-profile), rounded-polygon
//! and lobed splines, external (shaft) or internal (hub), on a selected shaft end or bore.
//!
//! The geometry lives in [`geom`]; this file reads the parameters, finds where to put the spline,
//! and sends ordinary SolidCraft commands (sketch, line, arc, spline, extrude cut).

pub mod geom;
pub mod place;

use geom::{Curve, Kind, Plan, Spec};
use place::{Placement, V3};
use serde_json::{Map, Value, json};

/// What the plugin needs from the host, so the logic can be tested without SolidCraft.
pub trait Host {
    type Error;
    fn run(&mut self, command: &str, args: &Value) -> Result<Value, Self::Error>;
    fn log(&mut self, _message: &str) {}
    fn progress(&mut self, _fraction: f32, _message: &str) {}
}

/// Why an entry failed.
#[derive(Debug)]
pub enum Failure<E> {
    /// A parameter is wrong, or the selection cannot be used.
    Input { field: String, reason: String },
    /// A host command failed; the host's own error is passed on unchanged.
    Host(E),
}

fn input<E>(field: &str, reason: impl Into<String>) -> Failure<E> {
    Failure::Input { field: field.into(), reason: reason.into() }
}

/// The host allows 256 engine calls per run; keep a margin for the placement queries.
const CALL_BUDGET: usize = 236;

/// Options that are not part of the profile itself.
#[derive(Clone, Debug)]
pub struct Options {
    pub length: f64,
    pub flip: bool,
    pub name: String,
    pub center: Option<V3>,
    pub axis: Option<V3>,
    pub body: Option<String>,
    pub lead: f64,
}

fn num(p: &Map<String, Value>, key: &str, default: f64) -> Result<f64, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => match v.as_f64() {
            Some(x) if x.is_finite() => Ok(x),
            _ => match v.as_str().map(str::trim) {
                Some("") => Ok(default),
                Some(s) => s.parse::<f64>().ok().filter(|x| x.is_finite()).ok_or_else(|| format!("`{key}` must be a number")),
                None => Err(format!("`{key}` must be a number")),
            },
        },
    }
}
fn opt(p: &Map<String, Value>, key: &str) -> Result<Option<f64>, String> {
    let v = num(p, key, 0.0)?;
    Ok((v != 0.0).then_some(v))
}
fn text<'a>(p: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// Reads the entry's parameters.
pub fn parse(params: &Value) -> Result<(Spec, Options), (String, String)> {
    let empty = Map::new();
    let p = match params {
        Value::Object(m) => m,
        Value::Null => &empty,
        _ => return Err(("params".into(), "parameters must be an object".into())),
    };
    fn wrap(k: &str) -> impl Fn(String) -> (String, String) + '_ {
        move |e| (k.to_string(), e)
    }
    let kind_s = text(p, "type").unwrap_or("involute");
    let kind = Kind::parse(kind_s).ok_or_else(|| ("type".to_string(), format!("unknown spline type `{kind_s}`")))?;
    let internal = match text(p, "gender").unwrap_or("external") {
        "external" | "shaft" => false,
        "internal" | "hub" => true,
        other => return Err(("gender".into(), format!("`{other}` is neither external nor internal"))),
    };
    let teeth = num(p, "teeth", 20.0).map_err(wrap("teeth"))?;
    if teeth.fract() != 0.0 || !(1.0..=500.0).contains(&teeth) {
        return Err(("teeth".into(), "must be a whole number from 1 to 500".into()));
    }
    let samples = num(p, "samples", 9.0).map_err(wrap("samples"))?;
    if !(3.0..=40.0).contains(&samples) {
        return Err(("samples".into(), "must be 3 to 40".into()));
    }
    let spec = Spec {
        kind,
        internal,
        count: teeth as u32,
        module: num(p, "module", 1.0).map_err(wrap("module"))?,
        pressure_angle: num(p, "pressure_angle", 30.0).map_err(wrap("pressure_angle"))?,
        profile_shift: num(p, "profile_shift", 0.0).map_err(wrap("profile_shift"))?,
        major: opt(p, "major_diameter").map_err(wrap("major_diameter"))?,
        minor: opt(p, "minor_diameter").map_err(wrap("minor_diameter"))?,
        width: num(p, "width", 6.0).map_err(wrap("width"))?,
        flank_angle: num(p, "flank_angle", 90.0).map_err(wrap("flank_angle"))?,
        corner_radius: num(p, "corner_radius", 1.0).map_err(wrap("corner_radius"))?,
        lobe_radius: opt(p, "lobe_radius").map_err(wrap("lobe_radius"))?,
        clearance: num(p, "clearance", 0.0).map_err(wrap("clearance"))?,
        rotation: num(p, "rotation", 0.0).map_err(wrap("rotation"))?,
        samples: samples as usize,
    };
    let length = num(p, "length", 20.0).map_err(wrap("length"))?;
    if length < 0.0 {
        return Err(("length".into(), "must be zero (through all) or positive".into()));
    }
    let name = text(p, "name").unwrap_or("Spline");
    if name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err(("name".into(), "at most 60 printable characters".into()));
    }
    let center = text(p, "center").map(|s| place::parse_v3(s).ok_or_else(|| ("center".to_string(), "write the centre as x, y, z in mm".to_string()))).transpose()?;
    let axis = text(p, "axis").map(|s| place::parse_axis(s).ok_or_else(|| ("axis".to_string(), "write the axis as X, Y, Z, -X, … or x, y, z".to_string()))).transpose()?;
    let options = Options {
        length,
        flip: p.get("flip").and_then(Value::as_bool).unwrap_or(false),
        name: name.to_string(),
        center,
        axis,
        body: text(p, "body").map(String::from),
        lead: 0.5,
    };
    Ok((spec, options))
}

fn sizes_json(spec: &Spec, plan: &Plan) -> Value {
    let s = &plan.sizes;
    json!({
        "type": spec.kind.id(),
        "gender": if spec.internal { "internal" } else { "external" },
        "count": spec.count,
        "major_diameter": s.major,
        "minor_diameter": s.minor,
        "pitch_diameter": s.pitch,
        "base_diameter": s.base,
        "pitch_thickness": s.thickness,
        "notes": s.notes,
    })
}

/// The `dimensions` entry: the computed sizes, without touching the design.
pub fn dimensions(params: &Value) -> Result<Value, (String, String)> {
    let (spec, _) = parse(params)?;
    let plan = geom::plan(&spec, None, None).map_err(|e| ("profile".to_string(), e))?;
    Ok(sizes_json(&spec, &plan))
}

fn curve_command(c: &Curve) -> (&'static str, Value) {
    match c {
        Curve::Line(points) => ("sketch.line", json!({ "points": points })),
        Curve::Arc { start, through, end } => ("sketch.arc.three_point", json!({ "start": start, "through": through, "end": end })),
        Curve::Spline { points, periodic } => ("sketch.spline.fit_point", json!({ "points": points, "periodic": periodic })),
        Curve::Circle { center, radius } => ("sketch.circle.center", json!({ "center": center, "radius": radius })),
    }
}

fn created_curves(reply: &Value) -> Vec<String> {
    let list = reply.get("created").and_then(|c| c.get("curves")).or_else(|| reply.get("curves"));
    list.and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// The `create` entry: places and cuts the spline.
pub fn create<H: Host>(host: &mut H, params: &Value) -> Result<Value, Failure<H::Error>> {
    let (spec, opts) = parse(params).map_err(|(f, r)| input(&f, r))?;
    // Check the profile before touching the design, so a bad size fails with a clear message.
    geom::plan(&spec, None, None).map_err(|e| input("profile", e))?;
    host.progress(0.05, "Finding the shaft or bore");
    let mut placement = place::locate(host, &opts, spec.internal)?;
    if opts.flip {
        placement.axis = place::scale(placement.axis, -1.0);
    }
    let plan = geom::plan(&spec, placement.radius, placement.outer_radius).map_err(|e| input("profile", e))?;
    let Placement { center, axis, ref body, .. } = placement;
    let (x_dir, y_dir) = place::frame(axis);
    let origin = place::add(center, place::scale(axis, -opts.lead));

    let total: usize = plan.stages.iter().map(|s| s.curves.len()).sum::<usize>() + 3 * plan.stages.len();
    let use_pattern = total > CALL_BUDGET;
    let mut features = Vec::new();
    let n = plan.stages.len();
    for (i, stage) in plan.stages.iter().enumerate() {
        let label = format!("{} {}", opts.name, stage.label);
        host.progress(0.1 + 0.8 * i as f32 / n as f32, &format!("Sketching {label}"));
        let sketch = host
            .run("sketch.create", &json!({ "plane": { "origin": origin, "x_dir": x_dir, "y_dir": y_dir }, "name": format!("{label} sketch") }))
            .map_err(Failure::Host)?;
        let sketch_id = sketch.get("sketch").cloned().unwrap_or_else(|| json!(format!("{label} sketch")));
        let patterned = use_pattern && stage.count > 1 && stage.loop_len.is_some();
        let curves = match (patterned, stage.loop_len) {
            (true, Some(len)) => &stage.curves[..len],
            _ => &stage.curves[..],
        };
        let mut ids = Vec::new();
        for c in curves {
            let (cmd, args) = curve_command(c);
            ids.extend(created_curves(&host.run(cmd, &args).map_err(Failure::Host)?));
        }
        if patterned {
            host.run("sketch.pattern.circular", &json!({ "entities": ids, "center": [0.0, 0.0], "count": stage.count })).map_err(Failure::Host)?;
        }
        host.run("sketch.finish", &json!({})).map_err(Failure::Host)?;
        host.progress(0.1 + 0.8 * (i as f32 + 0.5) / n as f32, &format!("Cutting {label}"));
        let mut cut = json!({ "sketch": sketch_id, "operation": "cut", "direction": "positive", "name": label });
        if opts.length > 0.0 {
            cut["distance"] = json!(opts.length + opts.lead);
        } else {
            cut["through_all"] = json!(true);
        }
        if let Some(b) = body {
            cut["targets"] = json!([b]);
        }
        host.run("solid.extrude", &cut).map_err(Failure::Host)?;
        features.push(label);
    }
    host.progress(1.0, "Done");
    let mut reply = sizes_json(&spec, &plan);
    reply["features"] = json!(features);
    reply["placement"] = json!({ "center": center, "axis": axis, "body": body, "length": if opts.length > 0.0 { json!(opts.length) } else { json!("through all") } });
    reply["patterned"] = json!(use_pattern);
    host.log(&format!("{} {} spline cut on {}", reply["gender"].as_str().unwrap_or(""), spec.kind.id(), body.as_deref().unwrap_or("the design")));
    Ok(reply)
}

#[cfg(target_arch = "wasm32")]
mod guest {
    wit_bindgen::generate!({ world: "command-plugin", path: "wit/plugin.wit" });

    use craft::plugin::types::{FieldFailure, PluginError};
    use craft::plugin::{commands, control, log};
    use exports::craft::plugin::entry::Guest;
    use serde_json::Value;

    struct Wasm;
    impl super::Host for Wasm {
        type Error = PluginError;
        fn run(&mut self, command: &str, args: &Value) -> Result<Value, PluginError> {
            if control::cancelled() {
                return Err(PluginError::Cancelled);
            }
            let reply = commands::run(command, &args.to_string())?;
            Ok(serde_json::from_str(&reply).unwrap_or(Value::Null))
        }
        fn log(&mut self, message: &str) {
            log::log(log::Level::Info, message);
        }
        fn progress(&mut self, fraction: f32, message: &str) {
            control::progress(fraction, message);
        }
    }

    fn bad(field: String, reason: String) -> PluginError {
        PluginError::InvalidInput(FieldFailure { field, reason })
    }

    struct Plugin;
    impl Guest for Plugin {
        fn execute(entry: String, params: String) -> Result<String, PluginError> {
            let params: Value = if params.trim().is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&params).map_err(|e| bad("params".into(), e.to_string()))?
            };
            let reply = match entry.as_str() {
                "create" => super::create(&mut Wasm, &params).map_err(|f| match f {
                    super::Failure::Input { field, reason } => bad(field, reason),
                    super::Failure::Host(e) => e,
                })?,
                "dimensions" => super::dimensions(&params).map_err(|(f, r)| bad(f, r))?,
                other => return Err(bad("entry".into(), format!("unknown entry `{other}`"))),
            };
            Ok(reply.to_string())
        }
    }

    export!(Plugin);
}
