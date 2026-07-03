//! Shader `// @prop name kind k=v ...` property schema + parser (pure data).
/// A typed property value; also used as the declared default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropValue {
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Color([f32; 4]),
    Int(i32),
    Bool(bool),
}
/// One shader-exposed property plus the metadata a future editor needs.
#[derive(Clone, Debug)]
pub struct Property {
    pub name: String,
    pub default: PropValue,
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub step: Option<f32>,
    pub label: Option<String>,
    pub group: Option<String>,
}
impl PropValue {
    /// The primary scalar of this value, for the single-slot params mapping
    /// (prop #i drives float slot i; bool → 0/1; multi-component → first lane).
    pub fn as_f32(self) -> f32 {
        match self {
            PropValue::Float(x) => x,
            PropValue::Int(i) => i as f32,
            PropValue::Bool(b) => if b { 1.0 } else { 0.0 },
            PropValue::Vec2(v) => v[0],
            PropValue::Vec3(v) => v[0],
            PropValue::Vec4(v) | PropValue::Color(v) => v[0],
        }
    }
}

/// The default params block (16 float slots): prop #i → slot i, in order.
pub fn default_params(props: &[Property]) -> [f32; 16] {
    let mut p = [0.0; 16];
    for (i, prop) in props.iter().take(16).enumerate() {
        p[i] = prop.default.as_f32();
    }
    p
}

/// Parse every `// @prop ...` annotation in `src`; malformed lines are skipped.
pub fn parse_props(src: &str) -> Vec<Property> {
    let mut props = Vec::new();
    for line in src.lines() {
        let Some(idx) = line.find("@prop") else { continue };
        let toks = tokenize(&line[idx + 5..]);
        let mut it = toks.iter();
        let (Some(name), Some(kind)) = (it.next(), it.next()) else { continue };
        let Some(default) = zero_for(kind) else { continue }; // unknown kind → skip
        let mut p = Property {
            name: name.clone(), default,
            min: None, max: None, step: None, label: None, group: None,
        };
        for kv in it {
            let Some((k, v)) = kv.split_once('=') else { continue };
            match k {
                "default" => if let Some(d) = parse_value(kind, v) { p.default = d },
                "min" => p.min = v.parse().ok(),
                "max" => p.max = v.parse().ok(),
                "step" => p.step = v.parse().ok(),
                "label" => p.label = Some(v.to_string()),
                "group" => p.group = Some(v.to_string()),
                _ => {}
            }
        }
        props.push(p);
    }
    props
}
/// Split on whitespace, keeping `"double quoted"` spans intact (quotes stripped).
fn tokenize(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() { out.push(std::mem::take(&mut cur)) }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() { out.push(cur) }
    out
}
fn zero_for(kind: &str) -> Option<PropValue> {
    Some(match kind {
        "float" => PropValue::Float(0.0),
        "vec2" => PropValue::Vec2([0.0; 2]),
        "vec3" => PropValue::Vec3([0.0; 3]),
        "vec4" => PropValue::Vec4([0.0; 4]),
        "color" => PropValue::Color([0.0, 0.0, 0.0, 1.0]),
        "int" => PropValue::Int(0),
        "bool" => PropValue::Bool(false),
        _ => return None,
    })
}
fn parse_value(kind: &str, raw: &str) -> Option<PropValue> {
    let floats = |r: &str| -> Vec<f32> {
        r.trim_matches(|c| matches!(c, '(' | ')' | '[' | ']'))
            .split(',').filter_map(|x| x.trim().parse().ok()).collect()
    };
    Some(match kind {
        "float" => PropValue::Float(raw.parse().ok()?),
        "int" => PropValue::Int(raw.parse().ok()?),
        "bool" => PropValue::Bool(raw == "true" || raw == "1"),
        "vec2" => { let f = floats(raw); PropValue::Vec2([*f.first()?, *f.get(1)?]) }
        "vec3" => { let f = floats(raw); PropValue::Vec3([*f.first()?, *f.get(1)?, *f.get(2)?]) }
        "vec4" => { let f = floats(raw); PropValue::Vec4([*f.first()?, *f.get(1)?, *f.get(2)?, *f.get(3)?]) }
        "color" => PropValue::Color(parse_color(raw)?),
        _ => return None,
    })
}
fn parse_color(raw: &str) -> Option<[f32; 4]> {
    let h = raw.strip_prefix('#')?;
    let byte = |i: usize| Some(u8::from_str_radix(h.get(i..i + 2)?, 16).ok()? as f32 / 255.0);
    Some([byte(0)?, byte(2)?, byte(4)?, if h.len() >= 8 { byte(6)? } else { 1.0 }])
}
