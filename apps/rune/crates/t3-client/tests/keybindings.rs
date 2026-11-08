//! Replays tests/fixtures/keybindings.json, produced by the web's own `resolveShortcutCommand`
//! over its default bindings plus two user overrides
//! (`node apps/rune/scripts/keybindings-reference.ts`).
use serde_json::Value;
use t3_client::keybindings::{KeyPress, decode, label, resolve};

#[test]
fn shortcuts_resolve_like_the_web() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/keybindings.json")).unwrap();
    let rules = decode(&fixture["rules"]);
    assert_eq!(rules.len(), fixture["rules"].as_array().unwrap().len());
    let mut failures = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let e = &case["event"];
        let press = KeyPress {
            key: e["key"].as_str().unwrap().into(),
            code: e["code"].as_str().map(Into::into),
            ctrl: e["ctrl"] == true,
            shift: e["shift"] == true,
            alt: e["alt"] == true,
            meta: e["meta"] == true,
        };
        let context = |name: &str| case["context"][name] == true;
        let got = resolve(&press, &rules, &context, false);
        let want = case["command"].as_str();
        if got != want {
            failures.push(format!("{}: rust {got:?} != web {want:?}", case["name"]));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(
        label(&rules, "thread.previous", false).as_deref(),
        Some("Ctrl Shift [")
    );
    assert_eq!(
        label(&rules, "commandPalette.toggle", false).as_deref(),
        Some("Ctrl Shift P")
    );
}
