//! The `create` entry against a recording host that answers like SolidCraft.

use serde_json::{Value, json};
use solidcraft_spline::{Failure, Host, create};

struct Mock {
    selection: Value,
    calls: Vec<(String, Value)>,
    curves: usize,
}

impl Host for Mock {
    type Error = String;
    fn run(&mut self, command: &str, args: &Value) -> Result<Value, String> {
        self.calls.push((command.to_string(), args.clone()));
        Ok(match command {
            "selection.get" => self.selection.clone(),
            "inspect.measure" => json!({"bodies": [{"name": "Shaft", "closed": true, "bbox": {"min": [-12, -12, -60], "max": [12, 12, 0]}}]}),
            "model.faces" => json!({"faces": [{"index": 3, "centroid": [0, 0, 0], "plane_normal": [0, 0, 1]}]}),
            "sketch.create" => json!({"sketch": 7, "name": args["name"]}),
            c if c.starts_with("sketch.") && c != "sketch.finish" && c != "sketch.pattern.circular" => {
                self.curves += 1;
                json!({"curves": [format!("x{}", self.curves)], "created": {"curves": [format!("x{}", self.curves)]}})
            }
            _ => json!({}),
        })
    }
}

fn mock(selection: Value) -> Mock {
    Mock { selection, calls: Vec::new(), curves: 0 }
}

fn edge_at_shaft_end() -> Value {
    json!({"count": 1, "items": [{"kind": "edge", "type": "edge", "body": "Shaft", "index": 0, "center": [0, 0, 0], "axis": [0, 0, 1], "radius": 12.0}]})
}

#[test]
fn external_involute_on_a_circular_edge() {
    let mut h = mock(edge_at_shaft_end());
    let reply = create(&mut h, &json!({"type": "involute", "gender": "external", "teeth": 20, "module": 1, "length": 25})).unwrap();
    assert_eq!(reply["major_diameter"], json!(21.0));
    let names: Vec<&str> = h.calls.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(names.iter().filter(|c| **c == "solid.extrude").count(), 2, "{names:?}");
    // The material is at -Z (bbox), so the sketch normal (x × y) must be -Z and the sketch sits 0.5 mm outside.
    let sk = &h.calls.iter().find(|(c, _)| c == "sketch.create").unwrap().1["plane"];
    let (x, y) = (&sk["x_dir"], &sk["y_dir"]);
    let f = |v: &Value, i: usize| v[i].as_f64().unwrap();
    let nz = f(x, 0) * f(y, 1) - f(x, 1) * f(y, 0);
    assert!((nz + 1.0).abs() < 1e-12, "normal z = {nz}");
    assert_eq!(sk["origin"], json!([0.0, 0.0, 0.5]));
    let cut = &h.calls.iter().filter(|(c, _)| c == "solid.extrude").last().unwrap().1;
    assert_eq!(cut["operation"], "cut");
    assert_eq!(cut["distance"], json!(25.5));
    assert_eq!(cut["targets"], json!(["Shaft"]));
    assert!(h.calls.len() <= 256);
}

#[test]
fn many_teeth_use_a_sketch_pattern() {
    let mut h = mock(edge_at_shaft_end());
    let reply = create(&mut h, &json!({"type": "involute", "gender": "internal", "teeth": 60, "module": 0.5, "length": 0})).unwrap();
    assert_eq!(reply["patterned"], json!(true));
    assert!(h.calls.iter().any(|(c, a)| c == "sketch.pattern.circular" && a["count"] == json!(60)));
    assert!(h.calls.iter().any(|(c, a)| c == "solid.extrude" && a["through_all"] == json!(true)));
    assert!(h.calls.len() <= 256, "{} calls", h.calls.len());
}

#[test]
fn flat_face_uses_its_centroid() {
    let sel = json!({"items": [{"kind": "face", "body": "Shaft", "index": 3, "surface": {"type": "plane", "normal": [0, 0, 1], "point": [5, 0, 0]}}]});
    let mut h = mock(sel);
    create(&mut h, &json!({"type": "rounded_polygon", "teeth": 6, "minor_diameter": 17, "corner_radius": 1.5})).unwrap();
    assert!(h.calls.iter().any(|(c, _)| c == "model.faces"));
}

#[test]
fn every_type_runs_both_ways() {
    let cases = [
        json!({"type": "involute"}),
        json!({"type": "straight", "teeth": 6, "minor_diameter": 23, "major_diameter": 26, "width": 6}),
        json!({"type": "serration", "teeth": 28, "minor_diameter": 18.4, "major_diameter": 20}),
        json!({"type": "polygon", "teeth": 3, "minor_diameter": 18, "major_diameter": 22}),
        json!({"type": "rounded_polygon", "teeth": 4, "minor_diameter": 14, "corner_radius": 2}),
        json!({"type": "lobed", "teeth": 6, "minor_diameter": 14.4, "major_diameter": 20}),
    ];
    for mut c in cases {
        for g in ["external", "internal"] {
            c["gender"] = json!(g);
            let mut h = mock(edge_at_shaft_end());
            create(&mut h, &c).unwrap_or_else(|e| panic!("{c}: {e:?}"));
        }
    }
}

#[test]
fn nothing_selected_explains_what_to_select() {
    let mut h = mock(json!({"count": 0, "items": []}));
    match create(&mut h, &json!({})) {
        Err(Failure::Input { field, reason }) => assert!(field == "selection" && reason.contains("circular edge"), "{reason}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn typed_center_and_axis_need_no_selection() {
    let mut h = mock(Value::Null);
    create(&mut h, &json!({"center": "10, 0, 5", "axis": "-Y"})).unwrap();
    assert!(!h.calls.iter().any(|(c, _)| c == "selection.get"));
}
