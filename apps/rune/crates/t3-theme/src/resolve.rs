//! Preference → painted palette, as apps/web/src/hooks/useTheme.ts `applyTheme` does through
//! `resolveThemeAppearance`, `resolveThemeHalf` and `applyThemePalette`.
//!
//! Not ported: the web also derives `--context-chip-*` colors (`contextChipVariables`) and applies
//! the contrast setting as a CSS `color-mix`. Both would be computed from `ResolvedTheme::colors`.

use serde::{Deserialize, Serialize};

use crate::{Appearance, ThemeColors, ThemeDefinition, builtin_themes, standard_colors};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppearanceMode {
    Light,
    Dark,
    #[default]
    System,
}

/// Theme ids to use for one appearance instead of [`ThemePreference::theme`]. An id that does not
/// resolve to a theme painting that appearance is ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Halves {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark: Option<String>,
}

impl Halves {
    pub fn get(&self, appearance: Appearance) -> Option<&str> {
        match appearance {
            Appearance::Light => self.light.as_deref(),
            Appearance::Dark => self.dark.as_deref(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemePreference {
    /// `"system"`, `"light"`, `"dark"`, `"t3-code"` or a theme id; legacy ids such as
    /// `"t3-grove"` are accepted. Anything unknown paints the stock theme.
    pub theme: String,
    pub mode: AppearanceMode,
    #[serde(default)]
    pub halves: Halves,
}

impl Default for ThemePreference {
    fn default() -> Self {
        Self {
            theme: "system".to_owned(),
            mode: AppearanceMode::System,
            halves: Halves::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTheme {
    /// The palette's id, or `"t3-code"` for the stock theme.
    pub id: String,
    pub label: String,
    pub appearance: Appearance,
    pub colors: ThemeColors,
    /// Whether the sidebar may show environment artwork: the stock theme and built-ins allow it,
    /// environment palettes do not.
    pub sidebar_artwork: bool,
}

/// Finds a theme by id, built-ins first, then `environment`. Legacy ids map to their built-in.
pub fn theme_definition<'a>(
    id: &str,
    environment: &'a [ThemeDefinition],
) -> Option<&'a ThemeDefinition> {
    let id = match id {
        "t3-chat-dark" => "t3-chat",
        "t3-grove" => "grove",
        "t3-ocean" => "ocean",
        "t3-ember" => "ember",
        "t3-iris" => "iris",
        id => id,
    };
    builtin_themes()
        .iter()
        .chain(environment)
        .find(|theme| theme.id == id)
}

pub fn resolve(
    pref: &ThemePreference,
    environment: &[ThemeDefinition],
    system_dark: bool,
) -> ResolvedTheme {
    let find = |id: &str| theme_definition(id, environment);
    // parseThemeHalves keeps a half only when it can paint its appearance.
    let half = |appearance: Appearance| {
        pref.halves
            .get(appearance)
            .and_then(find)
            .filter(|theme| theme.colors_for(appearance).is_some())
    };
    let wanted = match pref.mode {
        AppearanceMode::Light => Appearance::Light,
        AppearanceMode::Dark => Appearance::Dark,
        AppearanceMode::System if system_dark => Appearance::Dark,
        AppearanceMode::System => Appearance::Light,
    };
    let appearance = match (half(wanted), find(&pref.theme)) {
        (None, Some(theme)) if theme.colors_for(wanted).is_none() => theme.appearance,
        _ => wanted,
    };
    match half(appearance).or_else(|| find(&pref.theme)) {
        Some(theme) => ResolvedTheme {
            id: theme.id.clone(),
            label: theme.label.clone(),
            appearance,
            colors: theme
                .colors_for(appearance)
                .unwrap_or(&theme.colors)
                .clone(),
            sidebar_artwork: theme.sidebar_artwork,
        },
        None => ResolvedTheme {
            id: "t3-code".to_owned(),
            label: "T3 Code".to_owned(),
            appearance,
            colors: standard_colors(appearance).clone(),
            sidebar_artwork: true,
        },
    }
}
