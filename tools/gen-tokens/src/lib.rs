//! Design tokens: parse `design/tokens.toml` and render the Rust (PC) and
//! Kotlin (phone) token files. Output always uses `\n` line endings.

use toml::{Table, Value};

pub const COLOR_NAMES: [&str; 15] = [
    "surface",
    "surface_variant",
    "outline",
    "outline_strong",
    "text",
    "text_muted",
    "primary",
    "on_primary",
    "accent",
    "success",
    "warning",
    "danger",
    "success_text",
    "warning_text",
    "danger_text",
];
/// The four materials in order: native light, native dark, relay light, relay dark.
pub const MATERIAL_PATHS: [(&str, &str); 4] = [
    ("native", "light"),
    ("native", "dark"),
    ("relay", "light"),
    ("relay", "dark"),
];
const MATERIAL_KEYS: [&str; 6] = [
    "tint",
    "opacity",
    "blur",
    "highlight",
    "border_alpha",
    "grain",
];
pub const SPRING_NAMES: [&str; 4] = ["standard", "emphasised", "quick", "effects"];
pub const EASING_NAMES: [&str; 3] = ["standard", "decelerate", "accelerate"];
const TYPE_NAMES: [&str; 5] = ["caption", "body", "subtitle", "title", "display"];
const MOTION_NAMES: [&str; 3] = ["fast_ms", "normal_ms", "emphasised_ms"];

/// One theme's colours as 0xRRGGBB, in `COLOR_NAMES` order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub values: [u32; 15],
    pub primary: u32,
    pub outline: u32,
    pub outline_strong: u32,
}

/// One glass material: a tint colour, an opacity and the effects drawn on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub tint: u32,
    pub opacity: f32,
    pub blur: f32,
    pub highlight: f32,
    pub border_alpha: f32,
    pub grain: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringTok {
    pub stiffness: f32,
    pub damping_ratio: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    pub light: Palette,
    pub dark: Palette,
    pub space: Vec<u32>,
    pub radius: Vec<u32>,
    pub types: [u32; 5],
    pub motion: [u32; 3],
    pub materials: [Material; 4],
    pub elevation_blur: [f32; 4],
    pub elevation_alpha: [f32; 4],
    pub springs: [SpringTok; 4],
    pub easing: [[f32; 4]; 3],
}

fn table<'a>(root: &'a Table, name: &str) -> Result<&'a Table, String> {
    root.get(name)
        .and_then(Value::as_table)
        .ok_or_else(|| format!("missing table [{name}]"))
}

fn int(t: &Table, table_name: &str, key: &str) -> Result<u32, String> {
    t.get(key)
        .and_then(Value::as_integer)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| format!("missing or invalid [{table_name}] {key}"))
}

fn steps(root: &Table, name: &str, len: usize) -> Result<Vec<u32>, String> {
    let arr = table(root, name)?
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing [{name}] steps"))?;
    let v: Vec<u32> = arr
        .iter()
        .map(|x| x.as_integer().and_then(|i| u32::try_from(i).ok()))
        .collect::<Option<_>>()
        .ok_or_else(|| format!("invalid [{name}] steps"))?;
    if v.len() != len {
        return Err(format!(
            "[{name}] steps needs {len} values, has {}",
            v.len()
        ));
    }
    Ok(v)
}

fn palette(root: &Table, name: &str) -> Result<Palette, String> {
    let t = table(root, name)?;
    let mut values = [0u32; 15];
    for (slot, key) in values.iter_mut().zip(COLOR_NAMES) {
        let s = t
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing colour [{name}] {key}"))?;
        let hex = s.strip_prefix('#').filter(|h| h.len() == 6);
        *slot = hex
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .ok_or_else(|| format!("invalid colour [{name}] {key} = {s:?}, want \"#RRGGBB\""))?;
    }
    let at = |n: &str| values[COLOR_NAMES.iter().position(|c| *c == n).unwrap_or(0)];
    Ok(Palette {
        values,
        primary: at("primary"),
        outline: at("outline"),
        outline_strong: at("outline_strong"),
    })
}

fn hex(s: &str, what: &str) -> Result<u32, String> {
    s.strip_prefix('#')
        .filter(|h| h.len() == 6)
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .ok_or_else(|| format!("invalid colour {what} = {s:?}, want \"#RRGGBB\""))
}

/// A TOML number written as `0.8` or `380`.
fn float(t: &Table, ctx: &str, key: &str) -> Result<f32, String> {
    t.get(key)
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .map(|f| f as f32)
        .ok_or_else(|| format!("missing or invalid [{ctx}] {key}"))
}

fn floats<const N: usize>(t: &Table, ctx: &str, key: &str) -> Result<[f32; N], String> {
    let arr = t
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing [{ctx}] {key}"))?;
    let v: Vec<f32> = arr
        .iter()
        .map(|x| {
            x.as_float()
                .or_else(|| x.as_integer().map(|i| i as f64))
                .map(|f| f as f32)
        })
        .collect::<Option<_>>()
        .ok_or_else(|| format!("invalid [{ctx}] {key}"))?;
    v.try_into()
        .map_err(|v: Vec<f32>| format!("[{ctx}] {key} needs {N} values, has {}", v.len()))
}

fn material(root: &Table, look: &str, mode: &str) -> Result<Material, String> {
    let ctx = format!("material.{look}.{mode}");
    let t = table(root, "material")?
        .get(look)
        .and_then(Value::as_table)
        .and_then(|l| l.get(mode))
        .and_then(Value::as_table)
        .ok_or_else(|| format!("missing table [{ctx}]"))?;
    for k in MATERIAL_KEYS {
        if !t.contains_key(k) {
            return Err(format!("missing [{ctx}] {k}"));
        }
    }
    let tint = t.get("tint").and_then(Value::as_str).unwrap_or_default();
    Ok(Material {
        tint: hex(tint, &format!("[{ctx}] tint"))?,
        opacity: float(t, &ctx, "opacity")?,
        blur: float(t, &ctx, "blur")?,
        highlight: float(t, &ctx, "highlight")?,
        border_alpha: float(t, &ctx, "border_alpha")?,
        grain: float(t, &ctx, "grain")?,
    })
}

pub fn load(src: &str) -> Result<Tokens, String> {
    let root: Table = src.parse().map_err(|e| format!("tokens.toml: {e}"))?;
    // Palettes first so a colour typo is reported before anything else.
    let light = palette(&root, "light")?;
    let dark = palette(&root, "dark")?;
    let ty = table(&root, "type")?;
    let mut types = [0; 5];
    for (slot, k) in types.iter_mut().zip(TYPE_NAMES) {
        *slot = int(ty, "type", k)?;
    }
    let mo = table(&root, "motion")?;
    let mut motion = [0; 3];
    for (slot, k) in motion.iter_mut().zip(MOTION_NAMES) {
        *slot = int(mo, "motion", k)?;
    }
    let mut materials = [Material {
        tint: 0,
        opacity: 0.0,
        blur: 0.0,
        highlight: 0.0,
        border_alpha: 0.0,
        grain: 0.0,
    }; 4];
    for (slot, (look, mode)) in materials.iter_mut().zip(MATERIAL_PATHS) {
        *slot = material(&root, look, mode)?;
    }
    let el = table(&root, "elevation")?;
    let sp = table(&root, "spring")?;
    let mut springs = [SpringTok {
        stiffness: 0.0,
        damping_ratio: 0.0,
    }; 4];
    for (slot, name) in springs.iter_mut().zip(SPRING_NAMES) {
        let ctx = format!("spring.{name}");
        let t = sp
            .get(name)
            .and_then(Value::as_table)
            .ok_or_else(|| format!("missing table [{ctx}]"))?;
        *slot = SpringTok {
            stiffness: float(t, &ctx, "stiffness")?,
            damping_ratio: float(t, &ctx, "damping_ratio")?,
        };
    }
    let ez = table(&root, "easing")?;
    let mut easing = [[0.0f32; 4]; 3];
    for (slot, name) in easing.iter_mut().zip(EASING_NAMES) {
        *slot = floats::<4>(ez, "easing", name)?;
    }
    Ok(Tokens {
        materials,
        elevation_blur: floats::<4>(el, "elevation", "blur")?,
        elevation_alpha: floats::<4>(el, "elevation", "alpha")?,
        springs,
        easing,
        light,
        dark,
        space: steps(&root, "space", 6)?,
        radius: steps(&root, "radius", 4)?,
        types,
        motion,
    })
}

const HEADER: &str = "// Generated by tools/gen-tokens from design/tokens.toml. Do not edit.\n";

fn camel(snake: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in snake.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn pascal(s: &str) -> String {
    let c = camel(s);
    let mut cs = c.chars();
    cs.next()
        .map(|f| f.to_ascii_uppercase().to_string() + cs.as_str())
        .unwrap_or_default()
}

fn join(v: &[u32], suffix: &str) -> String {
    v.iter()
        .map(|x| format!("{x}{suffix}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A float as a Rust literal (`0.8`, `380.0`).
fn rf(v: f32) -> String {
    format!("{v:?}")
}

fn rf_list(v: &[f32]) -> String {
    v.iter().map(|x| rf(*x)).collect::<Vec<_>>().join(", ")
}

fn kf_list(v: &[f32]) -> String {
    v.iter()
        .map(|x| format!("{}f", rf(*x)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn material_const(look: &str, mode: &str) -> String {
    format!(
        "{}_{}",
        look.to_ascii_uppercase(),
        mode.to_ascii_uppercase()
    )
}

pub fn render_rust(t: &Tokens) -> String {
    let mut s = String::from(HEADER);
    s.push_str("\n/// Colours as 0xRRGGBB.\n#[derive(Clone, Copy)]\npub struct Colors {\n");
    for n in COLOR_NAMES {
        s.push_str(&format!("    pub {n}: u32,\n"));
    }
    s.push_str("}\n");
    for (name, p) in [("LIGHT", &t.light), ("DARK", &t.dark)] {
        s.push_str(&format!("\npub const {name}: Colors = Colors {{\n"));
        for (n, v) in COLOR_NAMES.iter().zip(p.values) {
            s.push_str(&format!("    {n}: 0x{v:06X},\n"));
        }
        s.push_str("};\n");
    }
    s.push_str(&format!(
        "\n/// Spacing steps in logical px.\npub const SPACE: [f32; {}] = [{}];\n",
        t.space.len(),
        join(&t.space, ".0")
    ));
    s.push_str(&format!(
        "\n/// Corner radii in logical px.\npub const RADIUS: [f32; {}] = [{}];\n",
        t.radius.len(),
        join(&t.radius, ".0")
    ));
    s.push_str("\n/// Type sizes in logical px.\npub mod ty {\n");
    for (n, v) in TYPE_NAMES.iter().zip(t.types) {
        s.push_str(&format!(
            "    pub const {}: f32 = {v}.0;\n",
            n.to_ascii_uppercase()
        ));
    }
    s.push_str("}\n\n/// Animation durations.\npub mod motion {\n");
    for (n, v) in MOTION_NAMES.iter().zip(t.motion) {
        s.push_str(&format!(
            "    pub const {}: u32 = {v};\n",
            n.to_ascii_uppercase()
        ));
    }
    s.push_str("}\n");
    s.push_str(
        "\n/// A glass material: the surface tint (0xRRGGBB), its opacity and the effects drawn on it. Blur is in logical px.\n\
         #[derive(Clone, Copy, Debug, PartialEq)]\n\
         pub struct Material {\n    pub tint: u32,\n    pub opacity: f32,\n    pub blur: f32,\n    pub highlight: f32,\n    pub border_alpha: f32,\n    pub grain: f32,\n}\n",
    );
    for ((look, mode), m) in MATERIAL_PATHS.iter().zip(t.materials) {
        s.push_str(&format!(
            "\npub const {}: Material = Material {{\n    tint: 0x{:06X},\n    opacity: {},\n    blur: {},\n    highlight: {},\n    border_alpha: {},\n    grain: {},\n}};\n",
            material_const(look, mode),
            m.tint,
            rf(m.opacity),
            rf(m.blur),
            rf(m.highlight),
            rf(m.border_alpha),
            rf(m.grain)
        ));
    }
    s.push_str(&format!(
        "\n/// Elevation steps: shadow blur in logical px and shadow alpha.\npub const ELEVATION_BLUR: [f32; 4] = [{}];\npub const ELEVATION_ALPHA: [f32; 4] = [{}];\n",
        rf_list(&t.elevation_blur),
        rf_list(&t.elevation_alpha)
    ));
    s.push_str(
        "\n/// A spring: stiffness and damping ratio.\n#[derive(Clone, Copy, Debug, PartialEq)]\npub struct Spring {\n    pub stiffness: f32,\n    pub damping_ratio: f32,\n}\n\n/// Springs; effects (opacity, colour) are critically damped and never overshoot.\npub mod spring {\n    use super::Spring;\n",
    );
    for (n, sp) in SPRING_NAMES.iter().zip(t.springs) {
        s.push_str(&format!(
            "    pub const {}: Spring = Spring {{\n        stiffness: {},\n        damping_ratio: {},\n    }};\n",
            n.to_ascii_uppercase(),
            rf(sp.stiffness),
            rf(sp.damping_ratio)
        ));
    }
    s.push_str("}\n\n/// Cubic-bezier easing curves (x1, y1, x2, y2).\npub mod easing {\n");
    for (n, c) in EASING_NAMES.iter().zip(t.easing) {
        s.push_str(&format!(
            "    pub const {}: [f32; 4] = [{}];\n",
            n.to_ascii_uppercase(),
            rf_list(&c)
        ));
    }
    s.push_str("}\n");
    s
}

pub fn render_kotlin(t: &Tokens) -> String {
    let mut s = String::from(HEADER);
    s.push_str(
        "package dev.phonebridge.app.ui\n\n\
         import androidx.compose.ui.graphics.Color\n\
         import androidx.compose.ui.unit.Dp\n\
         import androidx.compose.ui.unit.dp\n\
         import androidx.compose.ui.unit.sp\n\n\
         /** A glass material: tint, opacity and the effects drawn on it; blur in dp. */\n\
         class GlassMaterial(\n    val tint: Color,\n    val opacity: Float,\n    val blur: Float,\n    val highlight: Float,\n    val borderAlpha: Float,\n    val grain: Float,\n)\n\n\
         /** A spring: stiffness and damping ratio. */\n\
         class TokenSpring(val stiffness: Float, val dampingRatio: Float)\n\n\
         class TokenColors(\n",
    );
    for n in COLOR_NAMES {
        s.push_str(&format!("    val {}: Color,\n", camel(n)));
    }
    s.push_str(")\n\nobject Tokens {\n");
    for (name, p) in [("Light", &t.light), ("Dark", &t.dark)] {
        s.push_str(&format!("    val {name} = TokenColors(\n"));
        for (n, v) in COLOR_NAMES.iter().zip(p.values) {
            s.push_str(&format!("        {} = Color(0xFF{v:06X}),\n", camel(n)));
        }
        s.push_str("    )\n\n");
    }
    s.push_str(&format!(
        "    val Space: List<Dp> = listOf({})\n",
        join(&t.space, ".dp")
    ));
    s.push_str(&format!(
        "    val Radius: List<Dp> = listOf({})\n\n",
        join(&t.radius, ".dp")
    ));
    s.push_str("    object Type {\n");
    for (n, v) in TYPE_NAMES.iter().zip(t.types) {
        s.push_str(&format!("        val {} = {v}.sp\n", pascal(n)));
    }
    s.push_str("    }\n\n    object Motion {\n");
    for (n, v) in MOTION_NAMES.iter().zip(t.motion) {
        s.push_str(&format!(
            "        const val {} = {v}\n",
            n.to_ascii_uppercase()
        ));
    }
    s.push_str("    }\n\n    val Materials = listOf(\n");
    for ((look, mode), m) in MATERIAL_PATHS.iter().zip(t.materials) {
        s.push_str(&format!(
            "        GlassMaterial(Color(0xFF{:06X}), {}f, {}f, {}f, {}f, {}f), // {look} {mode}\n",
            m.tint,
            rf(m.opacity),
            rf(m.blur),
            rf(m.highlight),
            rf(m.border_alpha),
            rf(m.grain)
        ));
    }
    s.push_str("    )\n\n");
    s.push_str(&format!(
        "    val ElevationBlur: List<Float> = listOf({})\n    val ElevationAlpha: List<Float> = listOf({})\n\n    object Springs {{\n",
        kf_list(&t.elevation_blur),
        kf_list(&t.elevation_alpha)
    ));
    for (n, sp) in SPRING_NAMES.iter().zip(t.springs) {
        s.push_str(&format!(
            "        val {} = TokenSpring({}f, {}f)\n",
            pascal(n),
            rf(sp.stiffness),
            rf(sp.damping_ratio)
        ));
    }
    s.push_str("    }\n\n    object Easing {\n");
    for (n, c) in EASING_NAMES.iter().zip(t.easing) {
        s.push_str(&format!(
            "        val {}: List<Float> = listOf({})\n",
            pascal(n),
            kf_list(&c)
        ));
    }
    s.push_str("    }\n}\n");
    s
}

/// WCAG relative luminance of 0xRRGGBB.
fn luminance(rgb: u32) -> f64 {
    let lin = |shift: u32| {
        let c = f64::from((rgb >> shift) & 0xFF) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(16) + 0.7152 * lin(8) + 0.0722 * lin(0)
}

/// WCAG contrast ratio of two colours, 1.0 to 21.0.
pub fn contrast(a: u32, b: u32) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x >= y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// `tint` drawn at `opacity` over `backdrop`, as the colour the eye sees.
pub fn over(tint: u32, opacity: f32, backdrop: u32) -> u32 {
    let a = f64::from(opacity.clamp(0.0, 1.0));
    [16u32, 8, 0].iter().fold(0, |acc, &s| {
        let t = f64::from((tint >> s) & 0xFF);
        let b = f64::from((backdrop >> s) & 0xFF);
        acc | (((t * a + b * (1.0 - a)).round() as u32) << s)
    })
}

fn css_glass(m: &Material) -> String {
    format!(
        "  --glass-tint: #{:06X};\n  --glass-opacity: {};\n  --glass-blur: {}px;\n  --glass-highlight: {};\n  --glass-border-alpha: {};\n  --glass-grain: {};\n",
        m.tint,
        rf(m.opacity),
        rf(m.blur),
        rf(m.highlight),
        rf(m.border_alpha),
        rf(m.grain)
    )
}

fn css_colors(p: &Palette) -> String {
    let mut s = String::new();
    for (n, v) in COLOR_NAMES.iter().zip(p.values) {
        s.push_str(&format!("  --color-{}: #{v:06X};\n", n.replace('_', "-")));
    }
    s
}

/// The Floot stylesheet. `data-theme="dark"` and `data-look="relay"` pick the palette and material; the
/// `prefers-reduced-*` media queries and the `data-reduce-*` attributes (for the prototype's switches) apply
/// the same rules as `resolve` and `spring_or_cut`.
pub fn render_css(t: &Tokens) -> String {
    let mut s = String::from(
        "/* Generated by tools/gen-tokens from design/tokens.toml. Do not edit. */\n\n:root {\n",
    );
    s.push_str(&css_colors(&t.light));
    for (i, v) in t.space.iter().enumerate() {
        s.push_str(&format!("  --space-{}: {v}px;\n", i + 1));
    }
    for (i, v) in t.radius.iter().enumerate() {
        s.push_str(&format!("  --radius-{}: {v}px;\n", i + 1));
    }
    for (n, v) in TYPE_NAMES.iter().zip(t.types) {
        s.push_str(&format!("  --type-{n}: {v}px;\n"));
    }
    for (n, v) in MOTION_NAMES.iter().zip(t.motion) {
        s.push_str(&format!("  --motion-{}: {v}ms;\n", n.replace('_', "-")));
    }
    for (i, (b, a)) in t.elevation_blur.iter().zip(t.elevation_alpha).enumerate() {
        s.push_str(&format!(
            "  --elevation-{i}-blur: {}px;\n  --elevation-{i}-alpha: {};\n",
            rf(*b),
            rf(a)
        ));
    }
    for (n, sp) in SPRING_NAMES.iter().zip(t.springs) {
        s.push_str(&format!(
            "  --spring-{n}-stiffness: {};\n  --spring-{n}-damping: {};\n",
            rf(sp.stiffness),
            rf(sp.damping_ratio)
        ));
    }
    for (n, c) in EASING_NAMES.iter().zip(t.easing) {
        s.push_str(&format!("  --ease-{n}: cubic-bezier({});\n", rf_list(&c)));
    }
    s.push_str(&css_glass(&t.materials[0]));
    s.push_str("}\n\n[data-theme=\"dark\"] {\n");
    s.push_str(&css_colors(&t.dark));
    s.push_str(&css_glass(&t.materials[1]));
    s.push_str("}\n\n[data-look=\"relay\"] {\n");
    s.push_str(&css_glass(&t.materials[2]));
    s.push_str("}\n\n[data-look=\"relay\"][data-theme=\"dark\"] {\n");
    s.push_str(&css_glass(&t.materials[3]));
    s.push_str("}\n");
    // Reduced transparency (also high contrast and lite): opaque, no blur, grain or highlight.
    let opaque = "  --glass-opacity: 1 !important;\n  --glass-blur: 0px !important;\n  --glass-grain: 0 !important;\n  --glass-highlight: 0 !important;\n";
    s.push_str(&format!(
        "\n@media (prefers-reduced-transparency: reduce), (forced-colors: active) {{\n  :root:not([data-force-glass=\"true\"]), [data-look]:not([data-force-glass=\"true\"]) {{\n{opaque}  }}\n}}\n"
    ));
    s.push_str(&format!(
        "\n:root[data-reduce-transparency=\"true\"], :root[data-reduce-transparency=\"true\"][data-look] {{\n{opaque}}}\n"
    ));
    // Reduced motion: no springs, a short cross-fade instead.
    let cut = "  --motion-normal-ms: var(--motion-fast-ms) !important;
  --motion-emphasised-ms: var(--motion-fast-ms) !important;
";
    s.push_str(&format!(
        "\n@media (prefers-reduced-motion: reduce) {{\n  :root {{\n{cut}  }}\n}}\n"
    ));
    s.push_str(&format!(
        "\n:root[data-reduce-motion=\"true\"] {{\n{cut}}}\n"
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checkouts with `core.autocrlf` turn `\n` into `\r\n`; compare without that.
    fn lf(s: &str) -> String {
        s.replace("\r\n", "\n")
    }

    #[test]
    fn checked_in_rust_tokens_match_the_toml() {
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        assert_eq!(
            render_rust(&t),
            lf(include_str!("../../../crates/pb-tray/src/tokens.rs"))
        );
    }

    #[test]
    fn checked_in_kotlin_tokens_match_the_toml() {
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        assert_eq!(
            render_kotlin(&t),
            lf(include_str!(
                "../../../android/app/src/main/java/dev/phonebridge/app/ui/Tokens.kt"
            ))
        );
    }

    #[test]
    fn checked_in_css_matches_the_toml() {
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        assert_eq!(
            render_css(&t),
            lf(include_str!("../../../design/floot/base.css"))
        );
    }

    #[test]
    fn reduced_rules_only_touch_variables_that_exist() {
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        let css = render_css(&t);
        let root = css.split("[data-theme").next().unwrap();
        for line in css.lines().filter(|l| l.contains("!important")) {
            let name = line.trim().split(':').next().unwrap();
            assert!(
                root.contains(&format!("{name}:")),
                "{name} is not defined in :root"
            );
        }
        assert!(css.contains("--motion-normal-ms: var(--motion-fast-ms) !important;"));
    }

    #[test]
    fn contrast_of_black_on_white_is_21() {
        assert!((contrast(0x000000, 0xFFFFFF) - 21.0).abs() < 0.01);
        assert!((contrast(0xFFFFFF, 0xFFFFFF) - 1.0).abs() < 0.001);
    }

    #[test]
    fn over_blends_by_opacity() {
        assert_eq!(over(0xFFFFFF, 0.5, 0x000000), 0x808080);
        assert_eq!(over(0x123456, 1.0, 0xFFFFFF), 0x123456);
        assert_eq!(over(0x123456, 0.0, 0xFFFFFF), 0xFFFFFF);
    }

    /// The palette for the mode a material belongs to (materials are native light, native dark, relay light, relay dark).
    fn palette_of(t: &Tokens, material: usize) -> &Palette {
        if material.is_multiple_of(2) {
            &t.light
        } else {
            &t.dark
        }
    }

    fn colour(p: &Palette, name: &str) -> u32 {
        p.values[COLOR_NAMES.iter().position(|c| *c == name).unwrap()]
    }

    /// The glass over a pure white and a pure black backdrop: the worst cases for a translucent surface.
    fn worst_surfaces(m: &Material) -> [u32; 2] {
        [
            over(m.tint, m.opacity, 0xFFFFFF),
            over(m.tint, m.opacity, 0x000000),
        ]
    }

    #[test]
    fn glass_text_is_aa_over_white_and_black() {
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        for (i, m) in t.materials.iter().enumerate() {
            let p = palette_of(&t, i);
            for surface in worst_surfaces(m) {
                for name in ["text", "text_muted"] {
                    let c = contrast(colour(p, name), surface);
                    assert!(
                        c >= 4.5,
                        "{:?} {name} on {surface:06X}: {c:.2}",
                        MATERIAL_PATHS[i]
                    );
                }
            }
        }
    }

    #[test]
    fn icons_are_3_to_1() {
        // Icons are drawn in text, text_muted and primary. `outline` is a divider, not an icon colour.
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        for (i, m) in t.materials.iter().enumerate() {
            let p = palette_of(&t, i);
            for surface in worst_surfaces(m) {
                for name in ["text", "text_muted", "primary"] {
                    let c = contrast(colour(p, name), surface);
                    assert!(
                        c >= 3.0,
                        "{:?} {name} on {surface:06X}: {c:.2}",
                        MATERIAL_PATHS[i]
                    );
                }
            }
        }
    }

    #[test]
    fn every_mode_meets_aa() {
        // Reduced transparency, lite and high contrast resolve to the opaque tint (opacity 1).
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        for (i, m) in t.materials.iter().enumerate() {
            let p = palette_of(&t, i);
            for name in ["text", "text_muted"] {
                assert!(
                    contrast(colour(p, name), m.tint) >= 4.5,
                    "{:?} {name}",
                    MATERIAL_PATHS[i]
                );
            }
            for name in ["text", "text_muted", "primary"] {
                assert!(
                    contrast(colour(p, name), m.tint) >= 3.0,
                    "{:?} {name}",
                    MATERIAL_PATHS[i]
                );
            }
        }
    }

    #[test]
    fn control_boundaries_are_3_to_1_on_opaque_surfaces() {
        // WCAG 1.4.11: a boundary that identifies a control. Controls sit on opaque surfaces, never on glass.
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        for p in [&t.light, &t.dark] {
            for surface in ["surface", "surface_variant"] {
                let c = contrast(p.outline_strong, colour(p, surface));
                assert!(c >= 3.0, "outline_strong on {surface}: {c:.2}");
            }
            // Primary also appears as text on opaque surfaces.
            assert!(contrast(p.primary, colour(p, "surface")) >= 4.5);
        }
    }

    #[test]
    fn attention_is_readable_as_text_and_icon() {
        // The amber role carries words and an icon, so it must pass text contrast on the opaque surfaces.
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        for (mode, p) in [("light", &t.light), ("dark", &t.dark)] {
            for surface in ["surface", "surface_variant"] {
                let c = contrast(colour(p, "accent"), colour(p, surface));
                assert!(c >= 4.5, "accent on {surface} ({mode}): {c:.2}");
            }
        }
    }

    #[test]
    fn status_text_is_aa_on_its_tint() {
        // Badges are the status colour at 15% over the surface, with `<status>_text` drawn on top.
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        for (mode, p) in [("light", &t.light), ("dark", &t.dark)] {
            for status in ["success", "warning", "danger"] {
                for surface in ["surface", "surface_variant"] {
                    let tint = over(colour(p, status), 0.15, colour(p, surface));
                    let c = contrast(colour(p, &format!("{status}_text")), tint);
                    assert!(c >= 4.5, "{status}_text on its tint over {surface} ({mode}): {c:.2}");
                }
            }
        }
    }

    #[test]
    fn forced_glass_opts_out_of_the_os_preference() {
        // A prototype can show glass on a machine whose OS asks for reduced transparency, but only when it says so.
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        let css = render_css(&t);
        let block = css
            .split("@media (prefers-reduced-transparency: reduce)")
            .nth(1)
            .unwrap()
            .split("}\n}")
            .next()
            .unwrap();
        assert!(block.contains(":root:not([data-force-glass=\"true\"])"), "{block}");
        assert!(block.contains("[data-look]:not([data-force-glass=\"true\"])"), "{block}");
        // The explicit switch still wins.
        assert!(css.contains(":root[data-reduce-transparency=\"true\"]"));
    }

    #[test]
    fn primary_is_cobalt() {
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        assert_eq!(t.light.primary, 0x3651C5);
    }

    #[test]
    fn materials_load_for_both_looks_and_modes() {
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        assert_eq!(t.materials.len(), 4);
        let relay_dark = t.materials[3];
        assert!(relay_dark.opacity > 0.0 && relay_dark.opacity <= 1.0);
        assert!(relay_dark.blur > 0.0);
        // Native materials carry no highlight or grain: they are the platform's own look.
        assert_eq!(t.materials[0].grain, 0.0);
    }

    #[test]
    fn a_missing_material_key_is_an_error_naming_it() {
        let src =
            include_str!("../../../design/tokens.toml").replacen("border_alpha", "border_alfa", 1);
        let e = load(&src).unwrap_err();
        assert!(e.contains("border_alpha"), "{e}");
    }

    #[test]
    fn springs_and_easing_load_in_their_order() {
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        // Effects (opacity, colour) are critically damped: they never overshoot.
        assert_eq!(t.springs[3].damping_ratio, 1.0);
        assert!(t.springs.iter().all(|s| s.stiffness > 0.0));
        assert_eq!(t.easing[1], [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(t.elevation_blur[0], 0.0);
    }

    #[test]
    fn control_outlines_are_a_separate_stronger_colour() {
        let t = load(include_str!("../../../design/tokens.toml")).unwrap();
        assert_ne!(t.light.outline_strong, t.light.outline);
    }

    #[test]
    fn a_missing_colour_is_an_error_naming_it() {
        let e = load("[light]\nsurface = \"#ffffff\"\n").unwrap_err();
        assert!(e.contains("surface_variant"), "{e}");
    }
}
