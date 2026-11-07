//! Color math ported from apps/web/src/themePalette.ts. All internal math is f64 and rounds
//! exactly where the TypeScript does: 8-bit sRGB inside `oklch_to_rgb(_, true)` and the
//! 6/6/3-decimal OKLCH strings every derived role is stored as.

use crate::Srgb;

/// OKLCH as the web carries it: `h` in degrees, unnormalized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Oklch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

/// sRGB on the 0..255 scale, possibly fractional (the web's `ThemeRgbColor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Rgb {
    pub const fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b }
    }
}

pub(crate) fn mix(base: Rgb, overlay: Rgb, amount: f64) -> Rgb {
    Rgb {
        r: base.r + (overlay.r - base.r) * amount,
        g: base.g + (overlay.g - base.g) * amount,
        b: base.b + (overlay.b - base.b) * amount,
    }
}

/// WCAG relative luminance; linearizes with the 0.03928 threshold, unlike the OKLCH path.
pub(crate) fn luminance(color: Rgb) -> f64 {
    let linearize = |channel: f64| {
        let n = channel / 255.0;
        if n <= 0.03928 {
            n / 12.92
        } else {
            ((n + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linearize(color.r) + 0.7152 * linearize(color.g) + 0.0722 * linearize(color.b)
}

/// At luminance 0.179, black and white text have equal contrast.
pub(crate) fn is_dark(color: Rgb) -> bool {
    luminance(color) < 0.179
}

pub(crate) fn contrast(first: Rgb, second: Rgb) -> f64 {
    let (a, b) = (luminance(first), luminance(second));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn srgb_to_linear(channel: f64) -> f64 {
    let c = channel / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(channel: f64, round: bool) -> f64 {
    let c = if channel <= 0.0031308 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    };
    let srgb = c.clamp(0.0, 1.0) * 255.0;
    if round { srgb.round() } else { srgb }
}

/// The web's own sRGB → OKLCH (`themeRgbToOklch`), not culori's: the constants differ slightly.
pub(crate) fn rgb_to_oklch(color: Rgb) -> Oklch {
    let (r, g, b) = (
        srgb_to_linear(color.r),
        srgb_to_linear(color.g),
        srgb_to_linear(color.b),
    );
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    Oklch {
        l: lightness,
        c: a.hypot(bb),
        h: bb.atan2(a).to_degrees_js(),
    }
}

trait JsDegrees {
    fn to_degrees_js(self) -> f64;
}

impl JsDegrees for f64 {
    /// `(rad * 180) / Math.PI`, in that order; `f64::to_degrees` rounds differently.
    fn to_degrees_js(self) -> f64 {
        (self * 180.0) / std::f64::consts::PI
    }
}

fn oklch_to_linear(color: Oklch) -> [f64; 3] {
    let hr = (color.h * std::f64::consts::PI) / 180.0;
    let (a, bb) = (color.c * hr.cos(), color.c * hr.sin());
    let l = (color.l + 0.3963377774 * a + 0.2158037573 * bb).powf(3.0);
    let m = (color.l - 0.1055613458 * a - 0.0638541728 * bb).powf(3.0);
    let s = (color.l - 0.0894841775 * a - 1.291485548 * bb).powf(3.0);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
    ]
}

/// Reduces chroma by binary search until the color fits sRGB, keeping L and h.
pub(crate) fn gamut_map(color: Oklch) -> Oklch {
    let in_gamut = |c: f64| {
        oklch_to_linear(Oklch { c, ..color })
            .iter()
            .all(|ch| (-0.0001..=1.0001).contains(ch))
    };
    if in_gamut(color.c) {
        return color;
    }
    const RESOLUTION: f64 = 0.000001;
    let steps = (color.c.max(RESOLUTION).log2() - RESOLUTION.log2())
        .ceil()
        .max(1.0) as usize;
    let (mut low, mut high) = (0.0, color.c);
    for _ in 0..steps {
        let mid = (low + high) / 2.0;
        if in_gamut(mid) { low = mid } else { high = mid }
    }
    Oklch { c: low, ..color }
}

pub(crate) fn oklch_to_rgb(color: Oklch, round: bool) -> Rgb {
    let [r, g, b] = oklch_to_linear(gamut_map(color));
    Rgb::new(
        linear_to_srgb(r, round),
        linear_to_srgb(g, round),
        linear_to_srgb(b, round),
    )
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Direction {
    Lighter,
    Darker,
}

/// Moves only L until `base` reaches `min_contrast` against every surface, measured in 8-bit.
pub(crate) fn solve_lightness(
    base: Oklch,
    against: &[Rgb],
    min_contrast: f64,
    direction: Direction,
) -> Oklch {
    let contrast_of = |color: Oklch| {
        let rgb = oklch_to_rgb(color, true);
        against
            .iter()
            .map(|&surface| contrast(rgb, surface))
            .fold(f64::INFINITY, f64::min)
    };
    let lighter = direction == Direction::Lighter;
    let (mut low, mut high) = if lighter {
        (base.l, 1.0)
    } else {
        (0.0, base.l)
    };
    if contrast_of(base) >= min_contrast {
        return base;
    }
    for _ in 0..18 {
        let mid = (low + high) / 2.0;
        let passes = contrast_of(Oklch { l: mid, ..base }) >= min_contrast;
        if passes == lighter {
            high = mid
        } else {
            low = mid
        }
    }
    Oklch {
        l: if lighter { high } else { low },
        ..base
    }
}

/// `formatThemeColorNumber` read back as a number: JS `toFixed`, ties away from zero.
fn round_fixed(value: f64, digits: usize) -> f64 {
    if value.abs() < 10f64.powi(-(digits as i32)) / 2.0 {
        return 0.0;
    }
    // Rust rounds exact decimal ties to even; JS toFixed picks the larger magnitude.
    let wide = format!("{:.*}", digits + 25, value.abs());
    let (int, frac) = wide.split_once('.').expect("fixed format has a point");
    let tail = &frac[digits..];
    let magnitude: f64 = if tail.starts_with('5') && tail[1..].bytes().all(|b| b == b'0') {
        let scaled: u128 = format!("{int}{}", &frac[..digits]).parse().expect("digits");
        let up = (scaled + 1).to_string();
        let split = up.len().saturating_sub(digits);
        format!(
            "{}.{}",
            if split == 0 { "0" } else { &up[..split] },
            &up[split..]
        )
        .parse()
        .expect("decimal")
    } else {
        format!("{:.*}", digits, value.abs())
            .parse()
            .expect("decimal")
    };
    magnitude.copysign(value)
}

/// What the web paints for a role stored as `formatOklchThemeColor(color, alpha)`: the string is
/// rounded to 6/6/3 decimals, read back by culori (clamped), then resolved like `themeColorToHex`.
pub(crate) fn paint_canonical(color: Oklch, alpha: f64) -> Srgb {
    let hue = if color.c < 0.0000005 {
        0.0
    } else {
        ((color.h % 360.0) + 360.0) % 360.0
    };
    let stored = Oklch {
        l: round_fixed(color.l, 6).clamp(0.0, 1.0),
        c: round_fixed(color.c, 6).max(0.0),
        h: round_fixed(hue, 3),
    };
    let alpha = if alpha < 1.0 {
        round_fixed(alpha, 4).clamp(0.0, 1.0)
    } else {
        1.0
    };
    srgb_from(oklch_to_rgb(stored, true), quantize_alpha(alpha))
}

/// `themeOklchToThemeColor`.
pub(crate) fn from_oklch(color: Oklch) -> Srgb {
    paint_canonical(gamut_map(color), 1.0)
}

/// `themeRgbToThemeColor`.
pub(crate) fn from_rgb(color: Rgb) -> Srgb {
    paint_canonical(rgb_to_oklch(color), 1.0)
}

/// `toCanonicalThemeColor` followed by painting; `None` where the web drops the value.
pub(crate) fn canonical_color(value: &str) -> Option<Srgb> {
    parse_theme_color(value).map(|(color, alpha)| paint_canonical(color, alpha))
}

/// `parseThemeRgbColor`: the 8-bit color a CSS string resolves to, ignoring alpha.
pub(crate) fn parse_rgb(value: &str, fallback: Rgb) -> Rgb {
    parse_theme_color(value).map_or(fallback, |(color, _)| oklch_to_rgb(color, true))
}

fn quantize_alpha(alpha: f64) -> f64 {
    if alpha >= 1.0 {
        1.0
    } else {
        (alpha * 255.0).round() / 255.0
    }
}

fn srgb_from(rgb: Rgb, alpha: f64) -> Srgb {
    Srgb {
        r: (rgb.r / 255.0) as f32,
        g: (rgb.g / 255.0) as f32,
        b: (rgb.b / 255.0) as f32,
        a: alpha as f32,
    }
}

/// Parses a CSS color into the 8-bit color the web paints for it (`themeColorToHex`): through
/// OKLCH and chroma-reduced into sRGB, so `rgb(300 0 0)` is not simply clipped. Quantizing in f64
/// keeps exact .5 ties (`hsl(350 80% 50%)`) rounding the web's way.
///
/// Accepts hex (3/4/6/8 digits, `#` optional as in culori), `rgb()`/`rgba()` and
/// `hsl()`/`hsla()` in comma or space syntax, `oklch()`, `oklab()`, `black`, `white` and
/// `transparent`. The web also accepts the other CSS named colors, `lab()`, `lch()`, `hwb()` and
/// `color()`; those return `None` here.
pub fn parse_css_color(value: &str) -> Option<Srgb> {
    let (color, alpha) = parse_theme_color(value)?;
    Some(srgb_from(oklch_to_rgb(color, true), quantize_alpha(alpha)))
}

/// `parseThemeColor`: culori's parse, converted to OKLCH, clamped.
pub(crate) fn parse_theme_color(value: &str) -> Option<(Oklch, f64)> {
    let input = value.trim();
    to_theme_oklch(input, parse_culori(input)?)
}

fn to_theme_oklch(input: &str, parsed: (Parsed, Option<f64>)) -> Option<(Oklch, f64)> {
    let (color, alpha) = parsed;
    let color = culori_oklch(color);
    // Culori drops a `none` alpha; CSS treats a missing component as zero.
    let alpha = if ends_with_none_alpha(input) {
        0.0
    } else {
        alpha.unwrap_or(1.0)
    };
    if ![color.l, color.c, color.h, alpha]
        .iter()
        .all(|v| v.is_finite())
    {
        return None;
    }
    Some((
        Oklch {
            l: color.l.clamp(0.0, 1.0),
            c: color.c.max(0.0),
            h: color.h,
        },
        alpha.clamp(0.0, 1.0),
    ))
}

/// `/\/\s*none\s*\)$/i`
fn ends_with_none_alpha(input: &str) -> bool {
    let Some(rest) = input.strip_suffix(')') else {
        return false;
    };
    let rest = rest.trim_end();
    rest.len() >= 4
        && rest.is_char_boundary(rest.len() - 4)
        && rest[rest.len() - 4..].eq_ignore_ascii_case("none")
        && rest[..rest.len() - 4].trim_end().ends_with('/')
}

#[derive(Clone, Copy, Debug)]
enum Parsed {
    Rgb { r: f64, g: f64, b: f64 },
    Hsl { h: f64, s: f64, l: f64 },
    Oklab { l: f64, a: f64, b: f64 },
    Oklch { l: f64, c: f64, h: f64 },
}

fn rgb_family(parsed: (Parsed, Option<f64>)) -> Parsed {
    match parsed.0 {
        Parsed::Hsl { h, s, l } => hsl_to_rgb(h, s, l),
        other => other,
    }
}

fn normalize_hue(hue: f64) -> f64 {
    let h = hue % 360.0;
    if h < 0.0 { h + 360.0 } else { h }
}

/// culori's `converter("oklch")`, with its own constants and gray handling.
fn culori_oklch(parsed: Parsed) -> Oklch {
    let lab_to_lch = |l: f64, a: f64, b: f64| {
        let c = (a * a + b * b).sqrt();
        Oklch {
            l,
            c,
            h: if c != 0.0 {
                normalize_hue(b.atan2(a).to_degrees_js())
            } else {
                0.0
            },
        }
    };
    match parsed {
        Parsed::Oklch { l, c, h } => Oklch { l, c, h },
        Parsed::Oklab { l, a, b } => lab_to_lch(l, a, b),
        Parsed::Hsl { .. } | Parsed::Rgb { .. } => {
            let Parsed::Rgb { r, g, b } = rgb_family((parsed, None)) else {
                unreachable!()
            };
            let lin = |c: f64| {
                let abs = c.abs();
                if abs <= 0.04045 {
                    c / 12.92
                } else {
                    c.signum() * ((abs + 0.055) / 1.055).powf(2.4)
                }
            };
            let (lr, lg, lb) = (lin(r), lin(g), lin(b));
            let l_ =
                (0.412221469470763 * lr + 0.5363325372617348 * lg + 0.0514459932675022 * lb).cbrt();
            let m_ = (0.2119034958178252 * lr + 0.6806995506452344 * lg + 0.1073969535369406 * lb)
                .cbrt();
            let s_ = (0.0883024591900564 * lr + 0.2817188391361215 * lg + 0.6299787016738222 * lb)
                .cbrt();
            let l = 0.210454268309314 * l_ + 0.7936177747023054 * m_ - 0.0040720430116193 * s_;
            let (a, bb) = if r == g && g == b {
                (0.0, 0.0)
            } else {
                (
                    1.9779985324311684 * l_ - 2.4285922420485799 * m_ + 0.450593709617411 * s_,
                    0.0259040424655478 * l_ + 0.7827717124575296 * m_ - 0.8086757549230774 * s_,
                )
            };
            lab_to_lch(l, a, bb)
        }
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Parsed {
    let h = normalize_hue(h);
    let m1 = l + s * if l < 0.5 { l } else { 1.0 - l };
    let m2 = m1 - (m1 - l) * 2.0 * (((h / 60.0) % 2.0) - 1.0).abs();
    let lo = 2.0 * l - m1;
    let (r, g, b) = match (h / 60.0).floor() as i64 {
        0 => (m1, m2, lo),
        1 => (m2, m1, lo),
        2 => (lo, m1, m2),
        3 => (lo, m2, m1),
        4 => (m2, lo, m1),
        5 => (m1, lo, m2),
        _ => (lo, lo, lo),
    };
    Parsed::Rgb { r, g, b }
}

/// culori's `parse` for the syntaxes listed on [`parse_css_color`], in its parser order.
fn parse_culori(input: &str) -> Option<(Parsed, Option<f64>)> {
    if let Some(parsed) = tokenize(input).and_then(|tokens| parse_modern(&tokens)) {
        return Some(parsed);
    }
    parse_hex(input)
        .or_else(|| parse_legacy(input, "rgb"))
        .or_else(|| match input.to_ascii_lowercase().as_str() {
            "black" => Some((
                Parsed::Rgb {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                },
                None,
            )),
            "white" => Some((
                Parsed::Rgb {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                },
                None,
            )),
            _ => None,
        })
        .or_else(|| {
            (input == "transparent").then_some((
                Parsed::Rgb {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                },
                Some(0.0),
            ))
        })
        .or_else(|| parse_legacy(input, "hsl"))
}

fn parse_hex(input: &str) -> Option<(Parsed, Option<f64>)> {
    let digits = input.strip_prefix('#').unwrap_or(input);
    if !matches!(digits.len(), 3 | 4 | 6 | 8) || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |i: usize| u8::from_str_radix(&digits[i..=i], 16).expect("hex digit") as f64;
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).expect("hex digits") as f64;
    let (channels, alpha): ([f64; 3], Option<f64>) = match digits.len() {
        3 | 4 => (
            [0, 1, 2].map(|i| nibble(i) * 17.0),
            (digits.len() == 4).then(|| nibble(3) * 17.0),
        ),
        _ => ([0, 2, 4].map(byte), (digits.len() == 8).then(|| byte(6))),
    };
    let [r, g, b] = channels.map(|c| c / 255.0);
    Some((Parsed::Rgb { r, g, b }, alpha.map(|a| a / 255.0)))
}

/// `[+-]?\d*\.?\d+(?:[eE][+-]?\d+)?`, the number in culori's legacy regexes.
fn legacy_number(s: &str) -> Option<f64> {
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or(("", mantissa));
    let digits = |d: &str| d.bytes().all(|b| b.is_ascii_digit());
    let exponent_ok = exponent.is_none_or(|e| {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        !e.is_empty() && digits(e)
    });
    (!frac.is_empty() && digits(int) && digits(frac) && exponent_ok)
        .then(|| s.parse().ok())
        .flatten()
}

fn legacy_alpha(s: &str) -> Option<f64> {
    match s.strip_suffix('%') {
        Some(p) => legacy_number(p).map(|v| (v / 100.0).clamp(0.0, 1.0)),
        None => legacy_number(s).map(|v| v.clamp(0.0, 1.0)),
    }
}

/// culori's comma-syntax `rgb()`/`rgba()` and `hsl()`/`hsla()` regexes.
fn parse_legacy(input: &str, family: &str) -> Option<(Parsed, Option<f64>)> {
    let rest = input.strip_prefix(family)?;
    let inner = rest
        .strip_prefix("a(")
        .or_else(|| rest.strip_prefix('('))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let alpha = match parts.get(3) {
        Some(a) => Some(legacy_alpha(a)?),
        None => None,
    };
    let percent = |s: &str| s.strip_suffix('%').and_then(legacy_number);
    if family == "rgb" {
        let channels: Option<Vec<f64>> = if parts[0].ends_with('%') {
            parts[..3]
                .iter()
                .map(|p| percent(p).map(|v| v / 100.0))
                .collect()
        } else {
            parts[..3]
                .iter()
                .map(|p| legacy_number(p).map(|v| v / 255.0))
                .collect()
        };
        let [r, g, b] = channels?[..] else {
            return None;
        };
        return Some((Parsed::Rgb { r, g, b }, alpha));
    }
    let hue = parts[0];
    let h = ["deg", "grad", "rad", "turn"]
        .iter()
        .find_map(|unit| hue.strip_suffix(unit).map(|v| (v, *unit)))
        .map_or_else(
            || legacy_number(hue),
            |(v, unit)| {
                let v = legacy_number(v)?;
                Some(match unit {
                    "deg" => v,
                    "rad" => (v / std::f64::consts::PI) * 180.0,
                    "grad" => (v / 10.0) * 9.0,
                    _ => v * 360.0,
                })
            },
        )?;
    let s = (percent(parts[1])? / 100.0).clamp(0.0, 1.0);
    let l = (percent(parts[2])? / 100.0).clamp(0.0, 1.0);
    Some((Parsed::Hsl { h, s, l }, alpha))
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Function(String),
    Ident(String),
    Number(f64),
    Percentage(f64),
    Hue(f64),
    None,
    ParenClose,
    Alpha(Box<Tok>),
}

/// culori's CSS tokenizer. `None` covers both its `undefined` result and inputs where it throws.
fn tokenize(input: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = input.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let is_digit = |c: Option<char>| c.is_some_and(|c| c.is_ascii_digit());
    let ident_start =
        |c: Option<char>| c.is_some_and(|c| !c.is_ascii() || c.is_ascii_alphabetic() || c == '_');
    let ident_char = |c: Option<char>| {
        c.is_some_and(|c| !c.is_ascii() || c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    let is_ws = |c: Option<char>| matches!(c, Some(' ' | '\t' | '\n'));
    let is_num = |i: usize| match at(i) {
        Some('-' | '+') => is_digit(at(i + 1)) || (at(i + 1) == Some('.') && is_digit(at(i + 2))),
        Some('.') => is_digit(at(i + 1)),
        c => is_digit(c),
    };
    let is_ident = |i: usize| match at(i) {
        None => false,
        Some('-') => chars.len() - i >= 2 && (at(i + 1) == Some('-') || ident_start(at(i + 1))),
        c => ident_start(c),
    };
    let ident = |i: &mut usize| {
        let start = *i;
        while ident_char(at(*i)) {
            *i += 1;
        }
        chars[start..*i].iter().collect::<String>()
    };
    let digits = |i: &mut usize, value: &mut String| {
        while is_digit(at(*i)) {
            value.push(chars[*i]);
            *i += 1;
        }
    };
    let num = |i: &mut usize| -> Option<Tok> {
        let mut value = String::new();
        if let Some(sign @ ('-' | '+')) = at(*i) {
            value.push(sign);
            *i += 1;
        }
        digits(i, &mut value);
        if at(*i) == Some('.') && is_digit(at(*i + 1)) {
            value.push('.');
            *i += 1;
            digits(i, &mut value);
        }
        if matches!(at(*i), Some('e' | 'E')) {
            if matches!(at(*i + 1), Some('-' | '+')) && is_digit(at(*i + 2)) {
                value.extend([chars[*i], chars[*i + 1]]);
                *i += 2;
                digits(i, &mut value);
            } else if is_digit(at(*i + 1)) {
                value.push(chars[*i]);
                *i += 1;
                digits(i, &mut value);
            }
        }
        let v: f64 = value.parse().ok()?;
        if is_ident(*i) {
            let unit = match ident(i).as_str() {
                "deg" => 1.0,
                "rad" => 180.0 / std::f64::consts::PI,
                "grad" => 9.0 / 10.0,
                "turn" => 360.0,
                _ => return None,
            };
            return Some(Tok::Hue(v * unit));
        }
        if at(*i) == Some('%') {
            *i += 1;
            return Some(Tok::Percentage(v));
        }
        Some(Tok::Number(v))
    };

    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            ' ' | '\t' | '\n' => {
                while is_ws(at(i)) {
                    i += 1;
                }
            }
            ',' => return None,
            ')' => {
                tokens.push(Tok::ParenClose);
                i += 1;
            }
            '/' => {
                i += 1;
                while is_ws(at(i)) {
                    i += 1;
                }
                if is_num(i) {
                    match num(&mut i)? {
                        Tok::Hue(_) => return None,
                        alpha => tokens.push(Tok::Alpha(Box::new(alpha))),
                    }
                } else if is_ident(i) && ident(&mut i) == "none" {
                    tokens.push(Tok::Alpha(Box::new(Tok::None)));
                } else {
                    return None;
                }
            }
            '+' | '-' | '.' | '0'..='9' if is_num(i) => tokens.push(num(&mut i)?),
            '-' if is_ident(i) => tokens.push(Tok::Ident(ident(&mut i))),
            _ if ident_start(Some(ch)) => {
                let name = ident(&mut i);
                if at(i) == Some('(') {
                    i += 1;
                    tokens.push(Tok::Function(name));
                } else if name == "none" {
                    tokens.push(Tok::None);
                } else {
                    tokens.push(Tok::Ident(name));
                }
            }
            _ => return None,
        }
    }
    Some(tokens)
}

/// culori's `parseModernSyntax` plus the per-mode parsers for rgb, hsl, oklch and oklab.
fn parse_modern(tokens: &[Tok]) -> Option<(Parsed, Option<f64>)> {
    let (Tok::Function(name), rest) = tokens.split_first()? else {
        return None;
    };
    let mut coords = Vec::new();
    for (index, token) in rest.iter().enumerate() {
        match token {
            Tok::ParenClose if index + 1 < rest.len() => return None,
            Tok::ParenClose => {}
            Tok::None | Tok::Number(_) | Tok::Percentage(_) | Tok::Hue(_) | Tok::Alpha(_) => {
                coords.push(token.clone())
            }
            _ => return None,
        }
    }
    let alpha = match coords.len() {
        3 => Tok::None,
        4 => match coords.pop() {
            Some(Tok::Alpha(alpha)) => *alpha,
            _ => return None,
        },
        _ => return None,
    };
    if coords.iter().any(|c| matches!(c, Tok::Alpha(_))) {
        return None;
    }
    let alpha = match alpha {
        Tok::Number(v) => Some(v.clamp(0.0, 1.0)),
        Tok::Percentage(v) => Some((v / 100.0).clamp(0.0, 1.0)),
        _ => None,
    };
    // Missing (`none`) components are zero once converted.
    let value = |t: &Tok| match t {
        Tok::Number(v) | Tok::Percentage(v) | Tok::Hue(v) => *v,
        _ => 0.0,
    };
    let is = |t: &Tok, hue: bool| {
        if hue {
            matches!(t, Tok::Hue(_))
        } else {
            matches!(t, Tok::Percentage(_))
        }
    };
    // Lightness percentages are `/ 100`; chroma and a/b are `(v * 0.4) / 100`, in culori's order.
    let lightness = |t: &Tok| match t {
        Tok::Number(v) => v.clamp(0.0, 1.0),
        Tok::None => 0.0,
        other => (value(other) / 100.0).clamp(0.0, 1.0),
    };
    let chroma = |t: &Tok| match t {
        Tok::Number(v) => *v,
        Tok::None => 0.0,
        other => (value(other) * 0.4) / 100.0,
    };
    let [a, b, c] = &coords[..] else { return None };
    let parsed = match name.as_str() {
        "rgb" | "rgba" => {
            if coords.iter().any(|t| is(t, true)) {
                return None;
            }
            let ch = |t: &Tok| match t {
                Tok::Number(v) => v / 255.0,
                Tok::Percentage(v) => v / 100.0,
                _ => 0.0,
            };
            Parsed::Rgb {
                r: ch(a),
                g: ch(b),
                b: ch(c),
            }
        }
        "hsl" | "hsla" => {
            if is(a, false) || is(b, true) || is(c, true) {
                return None;
            }
            Parsed::Hsl {
                h: value(a),
                s: value(b) / 100.0,
                l: value(c) / 100.0,
            }
        }
        "oklch" => {
            if is(a, true) || is(c, false) {
                return None;
            }
            Parsed::Oklch {
                l: lightness(a),
                c: chroma(b).max(0.0),
                h: value(c),
            }
        }
        "oklab" => {
            if coords.iter().any(|t| is(t, true)) {
                return None;
            }
            Parsed::Oklab {
                l: lightness(a),
                a: chroma(b),
                b: chroma(c),
            }
        }
        _ => return None,
    };
    Some((parsed, alpha))
}
