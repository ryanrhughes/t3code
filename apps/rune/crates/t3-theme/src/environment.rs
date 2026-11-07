//! Themes a server publishes (`EnvironmentTheme` in packages/contracts/src/server.ts), turned into
//! palettes as `publishedThemeDefinitions` in apps/web/src/themePalette.ts does.

use serde_json::{Map, Value};

use crate::color::canonical_color;
use crate::{
    Appearance, Role, Srgb, ThemeDefinition, ThemeSource, Variants, default_colors,
    vivid_theme_colors,
};

/// Appearance keywords, built-ins and legacy aliases; a published theme with one of these ids
/// would capture stored preferences, so the web drops it.
const RESERVED_IDS: [&str; 13] = [
    "system",
    "light",
    "dark",
    "t3-chat",
    "grove",
    "ocean",
    "ember",
    "iris",
    "t3-chat-dark",
    "t3-grove",
    "t3-ocean",
    "t3-ember",
    "t3-iris",
];

/// Accepts the `EnvironmentTheme` JSON exactly as it arrives on the wire. Returns `None` when the
/// contracts schema rejects it, its id is reserved, or it carries neither seeds nor a usable color.
///
/// With both `canvas` and `accent`, its own appearance is generated from them; otherwise it starts
/// from the T3 Chat defaults. `colors` then overrides single roles, dropping unknown roles and
/// unparseable values. A variant is the defaults for that appearance plus its overrides.
pub fn environment_theme(theme: &Value) -> Option<ThemeDefinition> {
    let theme = theme.as_object()?;
    let id = theme
        .get("id")?
        .as_str()
        .filter(|id| valid_id(id) && !RESERVED_IDS.contains(id))?;
    if let Some(version) = theme.get("version") {
        version.as_f64().filter(|&v| v == 1.0)?;
    }
    let label = trimmed(theme.get("name")?, 48)?;
    let appearance = Appearance::from_name(theme.get("appearance")?.as_str()?)?;
    let seed = |key: &str| optional(theme, key, |v| v.as_str().filter(|s| valid_seed(s)));
    let (canvas, accent) = (seed("canvas")?, seed("accent")?);
    let colors = optional(theme, "colors", color_map)?;
    let variants = optional(theme, "variants", |v| {
        let v = v.as_object()?;
        Some([
            optional(v, "light", color_map)?,
            optional(v, "dark", color_map)?,
        ])
    })?;
    let variant = |a: Appearance| variants.as_ref().and_then(|v| v[a as usize].as_ref());

    let other = match appearance {
        Appearance::Light => Appearance::Dark,
        Appearance::Dark => Appearance::Light,
    };
    let seeded = canvas.zip(accent);
    let has_colors = [colors.as_ref(), variant(other)]
        .into_iter()
        .flatten()
        .any(|c| !overrides(c).is_empty());
    if seeded.is_none() && !has_colors {
        return None;
    }

    let palette = |for_appearance: Appearance, overrides_of: Option<&Map<String, Value>>| {
        let mut palette = match seeded {
            Some((canvas, accent)) if for_appearance == appearance => {
                vivid_theme_colors(appearance, canvas, accent)
            }
            _ => default_colors(for_appearance).clone(),
        };
        for (role, color) in overrides_of.map(overrides).unwrap_or_default() {
            palette.set(role, color);
        }
        palette
    };
    let mut definition_variants = Variants::default();
    if let Some(overrides_of) = variant(other) {
        *definition_variants.slot(other) = Some(palette(other, Some(overrides_of)));
    }
    Some(ThemeDefinition {
        id: id.to_owned(),
        label,
        appearance,
        colors: palette(appearance, colors.as_ref()),
        variants: definition_variants,
        sidebar_artwork: false,
        source: ThemeSource::Environment,
    })
}

/// `lenientThemeColorOverrides`.
fn overrides(colors: &Map<String, Value>) -> Vec<(Role, Srgb)> {
    colors
        .iter()
        .filter_map(|(role, value)| {
            Some((Role::from_name(role)?, canonical_color(value.as_str()?)?))
        })
        .collect()
}

/// An optional schema field: absent is `Some(None)`, present-but-invalid (including `null`) is `None`.
fn optional<'a, T>(
    object: &'a Map<String, Value>,
    key: &str,
    decode: impl FnOnce(&'a Value) -> Option<T>,
) -> Option<Option<T>> {
    match object.get(key) {
        None => Some(None),
        Some(value) => decode(value).map(Some),
    }
}

/// `Record(/^[a-zA-Z][a-zA-Z0-9]{0,63}$/, TrimmedNonEmptyString.check(isMaxLength(64)))`. Like
/// Effect's Record, entries whose key fails the pattern are dropped unchecked, while a bad value
/// under a valid key rejects the theme. Values come back trimmed, as the schema decodes them.
fn color_map(value: &Value) -> Option<Map<String, Value>> {
    value
        .as_object()?
        .iter()
        .filter(|(key, _)| {
            key.len() <= 64
                && key.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
                && key.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        .map(|(key, value)| Some((key.clone(), Value::String(trimmed(value, 64)?))))
        .collect()
}

/// `TrimmedNonEmptyString` with a max length in UTF-16 code units, as JS counts it.
fn trimmed(value: &Value, max: usize) -> Option<String> {
    let value = value.as_str()?.trim();
    (!value.is_empty() && value.encode_utf16().count() <= max).then(|| value.to_owned())
}

/// `/^[a-z0-9](?:[a-z0-9-]{0,47})$/`
fn valid_id(id: &str) -> bool {
    let lower = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    (1..=48).contains(&id.len())
        && lower(id.as_bytes()[0])
        && id.bytes().all(|b| lower(b) || b == b'-')
}

/// `/^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/`
fn valid_seed(seed: &str) -> bool {
    seed.strip_prefix('#')
        .is_some_and(|hex| matches!(hex.len(), 3 | 6) && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}
