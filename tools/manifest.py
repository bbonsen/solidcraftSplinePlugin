"""Writes package/spline/plugin.json and checks it against the SolidCraft manifest and dialog rules."""
import json, pathlib, sys

def num(title, default, desc, minimum=None, maximum=None, unit=None):
    p = {"type": "number", "title": title, "default": default, "description": desc}
    if minimum is not None: p["minimum"] = minimum
    if maximum is not None: p["maximum"] = maximum
    if unit: p["unit"] = unit
    return p

TYPES = [
    ("involute", "Involute (ISO 4156 / DIN 5480 style)"),
    ("straight", "Straight-sided (ISO 14)"),
    ("serration", "Serration (straight flanks)"),
    ("polygon", "Polygon P-profile (P3G / P4C style)"),
    ("rounded_polygon", "Rounded polygon (square, hex…)"),
    ("lobed", "Lobed / hexalobular"),
]
props = {
    "type": {"type": "string", "title": "Spline type", "default": "involute",
             "oneOf": [{"const": c, "title": t} for c, t in TYPES]},
    "gender": {"type": "string", "title": "Side", "default": "external",
               "oneOf": [{"const": "external", "title": "External (shaft)"}, {"const": "internal", "title": "Internal (hub / bore)"}]},
    "teeth": {"type": "integer", "title": "Teeth / splines / sides / lobes", "default": 20, "minimum": 2, "maximum": 200},
    "module": num("Module (involute)", 1.0, "Involute module m in mm.", 0.1, 20, "mm"),
    "pressure_angle": num("Pressure angle (involute)", 30.0, "30, 37.5 or 45 degrees for standard diameters.", 14.5, 50, "deg"),
    "profile_shift": num("Profile shift coefficient x (involute)", 0.0, "DIN 5480-style shift; moves the diameters by 2·x·m.", -1, 1),
    "major_diameter": num("Major diameter", 0.0, "0 = standard (involute). Needed for straight, serration, polygon and lobed.", 0, None, "mm"),
    "minor_diameter": num("Minor diameter / across flats", 0.0, "0 = standard (involute). Rounded polygon: across flats.", 0, None, "mm"),
    "width": num("Spline width B (straight)", 6.0, "Width of each straight-sided spline.", 0, None, "mm"),
    "flank_angle": num("Flank angle (serration)", 90.0, "Included angle between the two flanks of a tooth.", 20, 150, "deg"),
    "corner_radius": num("Corner radius (rounded polygon)", 1.0, "", 0, None, "mm"),
    "lobe_radius": num("Lobe radius (lobed)", 0.0, "0 = automatic (lobe and valley radii about equal).", 0, None, "mm"),
    "clearance": num("Fit clearance", 0.0, "Side clearance at the pitch circle (teeth) or on the diameter (outlines). External shrinks, internal grows.", 0, 5, "mm"),
    "length": num("Length", 20.0, "Spline length from the selected end; 0 = through all.", 0, None, "mm"),
    "rotation": num("Rotation", 0.0, "Turns the profile about the axis.", -360, 360, "deg"),
    "flip": {"type": "boolean", "title": "Flip direction", "default": False, "description": "Cut the other way along the axis."},
    "center": {"type": "string", "title": "Center x, y, z (optional)", "default": "", "maxLength": 100, "description": "Leave empty to use the selection."},
    "axis": {"type": "string", "title": "Axis (optional)", "default": "", "maxLength": 100, "description": "X, Y, Z, -X… or x, y, z. Used with Center."},
    "body": {"type": "string", "title": "Body (optional)", "default": "", "maxLength": 200, "description": "Only cut this body. Default: the selected body."},
    "name": {"type": "string", "title": "Feature name", "default": "Spline", "minLength": 1, "maxLength": 60},
    "samples": {"type": "integer", "title": "Fit points per involute flank", "default": 9, "minimum": 3, "maximum": 40},
}
schema = {"type": "object", "required": ["type", "gender", "teeth"], "properties": props}

READS = ["selection.get", "model.faces", "inspect.measure"]
EDITS = ["sketch.create", "sketch.line", "sketch.arc.three_point", "sketch.spline.fit_point", "sketch.circle.center",
         "sketch.pattern.circular", "sketch.finish", "solid.extrude"]
manifest = {
    "id": "spline",
    "name": "Splines",
    "version": "0.1.0",
    "description": "Cuts involute, straight-sided, serrated, polygon (P3/P4), rounded-polygon and lobed splines on a shaft (external) or in a hub (internal). Select a circular edge, then run.",
    "capabilities": ["commands@0.1"],
    "runtime": {
        "module": "spline.wasm",
        "entries": [
            {"name": "create", "effect": "edit", "input_schema": schema,
             "slots": [{"kind": "ribbon", "name": "Model"}]},
            {"name": "dimensions", "effect": "read", "input_schema": schema},
        ],
        "commands": sorted(READS + EDITS),
        "limits": {"fuel": 200000000, "memory_mib": 64, "deadline_ms": 30000},
    },
}

# Checks mirroring craft-plugin's manifest parser and the SolidCraft plugin dialog.
text = json.dumps(manifest, ensure_ascii=False, separators=(",", ":")) + "\n"
assert len(text.encode()) <= 16 * 1024, len(text.encode())
assert len(manifest["name"]) <= 80 and len(manifest["description"]) <= 240
ANN = {"type", "title", "description", "default", "unit", "x-unit", "units", "examples", "$comment", "deprecated"}
ALLOWED = {"number": {"minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "multipleOf"},
           "integer": {"minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"},
           "boolean": set(), "string": {"minLength", "maxLength", "format"}}
for k, p in props.items():
    allowed = {"oneOf", "enum"} if ("oneOf" in p or "enum" in p) else ALLOWED[p["type"]]
    extra = set(p) - ANN - allowed
    assert not extra, (k, extra)
    if "oneOf" in p:
        assert all(set(o) <= {"const", "title", "description"} for o in p["oneOf"])
    assert k not in {"selection", "target", "targets", "bodies", "body_ids"}
assert all(len(c) <= 128 for c in manifest["runtime"]["commands"])

out = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "package/spline/plugin.json")
out.write_text(text, encoding="utf-8")
print(f"wrote {out} ({len(text.encode())} bytes)")
