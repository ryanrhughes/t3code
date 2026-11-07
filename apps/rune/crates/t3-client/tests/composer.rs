//! Replays tests/fixtures/composer.json, produced by the web's real `detectComposerTrigger` and
//! `serializeComposerFileLink` (`node apps/rune/scripts/composer-reference.ts`).
use serde_json::Value;
use t3_client::composer::{TriggerKind, detect, file_link};

/// The byte offset of a UTF-16 offset in `text`.
fn byte_of(text: &str, utf16: u64) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= utf16 as usize {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}

fn kind(name: &str) -> TriggerKind {
    match name {
        "path" => TriggerKind::Path,
        "pull-request" => TriggerKind::PullRequest,
        "slash-command" => TriggerKind::SlashCommand,
        "slash-model" => TriggerKind::SlashModel,
        "skill" => TriggerKind::Skill,
        other => panic!("unknown trigger kind {other}"),
    }
}

#[test]
fn composer_triggers_match_the_web() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/composer.json")).unwrap();
    let mut failures = Vec::new();
    for case in fixture["triggers"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        let cursor = byte_of(text, case["cursor"].as_u64().unwrap());
        let got = detect(text, cursor).map(|t| (t.kind, t.query, t.start, t.end));
        let want = case["trigger"].as_object().map(|t| {
            (
                kind(t["kind"].as_str().unwrap()),
                t["query"].as_str().unwrap().to_owned(),
                byte_of(text, t["rangeStart"].as_u64().unwrap()),
                byte_of(text, t["rangeEnd"].as_u64().unwrap()),
            )
        });
        if got != want {
            failures.push(format!("{text:?} @ {cursor}: rust {got:?} != web {want:?}"));
        }
    }
    for case in fixture["links"].as_array().unwrap() {
        let (path, want) = (
            case["path"].as_str().unwrap(),
            case["link"].as_str().unwrap(),
        );
        if file_link(path) != want {
            failures.push(format!(
                "link {path:?}: rust {:?} != web {want:?}",
                file_link(path)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
