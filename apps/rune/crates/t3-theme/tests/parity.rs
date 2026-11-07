//! Replays tests/fixtures/reference.json, produced by the web's real derivation
//! (`node apps/rune/scripts/theme-reference.ts`), and requires byte-equal 8-bit sRGB per role.

use std::sync::OnceLock;

use serde_json::Value;
use t3_theme::{
    Appearance, AppearanceMode, Halves, Role, ThemeColors, ThemeDefinition, ThemePreference,
    builtin_themes, environment_theme, parse_css_color, resolve, vivid_theme_colors,
};

fn fixture() -> &'static Value {
    static FIXTURE: OnceLock<Value> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        serde_json::from_str(include_str!("fixtures/reference.json")).expect("fixture")
    })
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} in {value}"))
}

fn appearance(value: &Value) -> Appearance {
    Appearance::from_name(value.as_str().expect("appearance")).expect("light or dark")
}

/// Every role mismatch, as `context role: rust != web`.
fn diff_colors(
    context: &str,
    actual: &ThemeColors,
    expected: &Value,
    mismatches: &mut Vec<String>,
) {
    for role in Role::ALL {
        let want = str_of(expected, role.name());
        let got = actual.get(role).to_hex();
        if got != want {
            mismatches.push(format!("{context} {}: {got} != {want}", role.name()));
        }
    }
}

fn assert_none(mismatches: Vec<String>) {
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

fn environment_definitions() -> Vec<ThemeDefinition> {
    fixture()["environment"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|case| environment_theme(&case["theme"]))
        .collect()
}

#[test]
fn environment_themes_match_the_web() {
    let mut mismatches = Vec::new();
    for case in fixture()["environment"].as_array().unwrap() {
        let label = str_of(case, "label");
        let actual = environment_theme(&case["theme"]);
        let expected = &case["definition"];
        let Some(actual) = actual else {
            assert!(
                expected.is_null(),
                "{label}: Rust rejected a theme the web accepts"
            );
            continue;
        };
        assert!(
            !expected.is_null(),
            "{label}: Rust accepted a theme the web rejects"
        );
        assert_eq!(actual.id, str_of(expected, "id"), "{label}");
        assert_eq!(actual.label, str_of(expected, "label"), "{label}");
        assert_eq!(
            actual.appearance,
            appearance(&expected["appearance"]),
            "{label}"
        );
        diff_colors(label, &actual.colors, &expected["colors"], &mut mismatches);
        for other in Appearance::ALL
            .into_iter()
            .filter(|&a| a != actual.appearance)
        {
            let want = &expected["variants"][other.name()];
            match actual.variants.get(other) {
                Some(colors) => diff_colors(
                    &format!("{label}/{}", other.name()),
                    colors,
                    want,
                    &mut mismatches,
                ),
                None => assert!(want.is_null(), "{label}: missing {} variant", other.name()),
            }
        }
    }
    assert_none(mismatches);
}

#[test]
fn vivid_palettes_match_the_web() {
    let mut mismatches = Vec::new();
    for case in fixture()["vivid"].as_array().unwrap() {
        let (canvas, accent) = (str_of(case, "canvas"), str_of(case, "accent"));
        let colors = vivid_theme_colors(appearance(&case["appearance"]), canvas, accent);
        diff_colors(
            &format!("{canvas}/{accent}"),
            &colors,
            &case["colors"],
            &mut mismatches,
        );
    }
    assert_none(mismatches);
}

#[test]
fn css_colors_parse_like_the_web() {
    let mut mismatches = Vec::new();
    for case in fixture()["parse"].as_array().unwrap() {
        let input = str_of(case, "input");
        let got = parse_css_color(input).map(|c| c.to_hex());
        let want = case["hex"].as_str().map(str::to_owned);
        if got != want {
            mismatches.push(format!("{input:?}: {got:?} != {want:?}"));
        }
    }
    assert_none(mismatches);
}

#[test]
fn preferences_resolve_like_the_web() {
    let environment = environment_definitions();
    let mut mismatches = Vec::new();
    for case in fixture()["resolve"].as_array().unwrap() {
        let preference = &case["preference"];
        let halves = &preference["halves"];
        let pref = ThemePreference {
            theme: str_of(preference, "theme").to_owned(),
            mode: serde_json::from_value::<AppearanceMode>(preference["mode"].clone()).unwrap(),
            halves: Halves {
                light: halves["light"].as_str().map(str::to_owned),
                dark: halves["dark"].as_str().map(str::to_owned),
            },
        };
        let system_dark = case["systemDark"].as_bool().unwrap();
        let context = format!("{pref:?} system_dark={system_dark}");
        let expected = &case["expected"];
        let actual = resolve(&pref, &environment, system_dark);
        assert_eq!(
            actual.id,
            expected["id"].as_str().unwrap_or("t3-code"),
            "{context}"
        );
        assert_eq!(
            actual.appearance,
            appearance(&expected["appearance"]),
            "{context}"
        );
        assert_eq!(
            actual.sidebar_artwork,
            expected["sidebarArtwork"].as_bool().unwrap(),
            "{context}"
        );
        diff_colors(
            &context,
            &actual.colors,
            &expected["colors"],
            &mut mismatches,
        );
    }
    assert_none(mismatches);
}

#[test]
fn builtins_are_the_shared_palettes() {
    let ids: Vec<&str> = builtin_themes().iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["t3-chat", "grove", "ocean", "ember", "iris"]);
    for theme in builtin_themes() {
        assert_eq!(
            theme.appearances(),
            [Appearance::Light, Appearance::Dark],
            "{}",
            theme.id
        );
        assert!(theme.sidebar_artwork);
    }
}

#[test]
fn role_names_round_trip() {
    for (index, role) in Role::ALL.into_iter().enumerate() {
        assert_eq!(role as usize, index);
        assert_eq!(Role::from_name(role.name()), Some(role));
    }
    assert_eq!(Role::from_name("bogus"), None);
}

#[test]
fn preferences_persist_as_json() {
    let pref = ThemePreference {
        theme: "grove".to_owned(),
        mode: AppearanceMode::System,
        halves: Halves {
            light: None,
            dark: Some("omarchy".to_owned()),
        },
    };
    let json = serde_json::to_value(&pref).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "theme": "grove", "mode": "system", "halves": { "dark": "omarchy" } })
    );
    assert_eq!(
        serde_json::from_value::<ThemePreference>(json).unwrap(),
        pref
    );
}
