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
/// One shader-exposed property plus the metadata the editor needs.
#[derive(Clone, Debug)]
pub struct Property {
    pub name: String,
    pub default: PropValue,
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub step: Option<f32>,
    pub label: Option<String>,
    pub group: Option<String>,
    /// Named values for a discrete prop: `choices="Soft,Normal,Hard"` turns an
    /// `int` into a picker whose entry *i* sends `i`.
    ///
    /// Display only — the shader still reads one float in one slot. Empty for
    /// every other prop.
    pub choices: Vec<String>,
}
impl PropValue {
    /// The kind, spelled exactly as an author writes it in `@prop`.
    ///
    /// So a consumer describing a variable to somebody else — the settings panel,
    /// the gRPC `Shader` service — names it the way the shader that declared it
    /// does, rather than inventing a second vocabulary for the same six kinds.
    pub fn kind(self) -> &'static str {
        match self {
            PropValue::Float(_) => "float",
            PropValue::Int(_) => "int",
            PropValue::Bool(_) => "bool",
            PropValue::Vec2(_) => "vec2",
            PropValue::Vec3(_) => "vec3",
            PropValue::Vec4(_) => "vec4",
            PropValue::Color(_) => "color",
        }
    }

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

/// Fold one source's props into a bundle-wide union: append what is new, keep
/// what is already there. FIRST DEFINITION WINS.
///
/// Two callers build this union — `shader.pipeline` while compiling, `shader.load`
/// from disk for a bundle that is not loaded — and any disagreement between them
/// silently mis-slots every edited variable, so the rule lives in one place.
pub fn merge_props(into: &mut Vec<Property>, from: Vec<Property>) {
    for p in from {
        if !into.iter().any(|q| q.name == p.name) {
            into.push(p);
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
            choices: Vec::new(),
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
                // `choices="Soft,Normal,Hard"` — the quotes are already stripped by
                // `tokenize`, so a label may contain spaces but not a comma.
                "choices" => {
                    p.choices =
                        v.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect()
                }
                _ => {}
            }
        }
        props.push(p);
    }
    props
}
/// Rewrite a shader's `@optimized` quality knobs to their cheap values.
///
/// A shader opts into the per-world Optimized toggle by annotating a module
/// constant with the value to use when the toggle is on:
///
/// ```wgsl
/// const FBM_OCTAVES: i32 = 5;   // @optimized 2
/// ```
///
/// Only the literal changes, so the reference source stays the single definition
/// of the shader — there is no second file to keep in step — and naga folds the
/// constant through loop bounds and branches when it compiles the variant.
///
/// This is the one textual preprocessing step in the runtime shader path, and it
/// is deliberately dumb: a line whose annotation is not a bare number is left
/// exactly as written, so prose mentioning `@optimized` cannot corrupt a
/// constant.
pub fn apply_optimized(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        match rewrite_optimized(line) {
            Some(l) => out.push_str(&l),
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// Whether `src` declares at least one `@optimized` knob — i.e. whether this
/// shader HAS a cheap variant, and so whether the settings toggle should be live
/// for it rather than greyed out.
pub fn has_optimized(src: &str) -> bool {
    src.lines().any(|l| rewrite_optimized(l).is_some())
}

/// One line: `const NAME: T = <lit>;` + `@optimized <value>` → the same line with
/// `<lit>` replaced. `None` when the line is not a well-formed annotation.
fn rewrite_optimized(line: &str) -> Option<String> {
    let at = line.find("@optimized")?;
    let value = line[at + "@optimized".len()..].split_whitespace().next()?;
    value.parse::<f64>().ok()?; // prose, not a knob — leave the line alone
    let eq = line[..at].find('=')?;
    let semi = eq + line[eq..at].find(';')?;
    Some(format!("{}= {}{}", &line[..eq], value, &line[semi..]))
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
