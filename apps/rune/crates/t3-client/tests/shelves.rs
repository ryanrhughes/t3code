//! Replays tests/fixtures/shelves.json, produced by the web's own sidebar rules
//! (`node apps/rune/scripts/shelves-reference.ts`).
use serde_json::Value;
use t3_client::protocol::ThreadShell;
use t3_client::shelves::{
    Notice, NotificationGate, has_unseen_completion, is_snoozed, iso_ms, observe, sidebar_status,
    sort_pinned, thread_woke_at,
};

#[test]
fn sidebar_rules_match_the_web() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/shelves.json")).unwrap();
    let now = iso_ms(fixture["now"].as_str().unwrap()).unwrap();
    let shell = |v: &Value| serde_json::from_value::<ThreadShell>(v.clone()).unwrap();
    let mut failures = Vec::new();
    for case in fixture["snoozed"].as_array().unwrap() {
        let (got, want) = (
            is_snoozed(&shell(&case["shell"]), now),
            case["snoozed"].as_bool().unwrap(),
        );
        if got != want {
            failures.push(format!(
                "snoozed {}: rust {got} != web {want}",
                case["name"]
            ));
        }
    }
    for case in fixture["snoozed"].as_array().unwrap() {
        let got = thread_woke_at(&shell(&case["shell"]), now);
        let want = case["woke"].as_str().map(str::to_owned);
        if got != want {
            failures.push(format!(
                "woke {}: rust {got:?} != web {want:?}",
                case["name"]
            ));
        }
    }
    for case in fixture["unread"].as_array().unwrap() {
        let (got, want) = (
            has_unseen_completion(&shell(&case["shell"])),
            case["unread"].as_bool().unwrap(),
        );
        if got != want {
            failures.push(format!("unread {}: rust {got} != web {want}", case["name"]));
        }
    }
    for case in fixture["status"].as_array().unwrap() {
        let (got, want) = (
            sidebar_status(&shell(&case["shell"])),
            case["status"].as_str().unwrap(),
        );
        if got != want {
            failures.push(format!("status {}: rust {got} != web {want}", case["name"]));
        }
    }
    for case in fixture["gate"].as_array().unwrap() {
        let mut gate = NotificationGate::default();
        let mut prior = None;
        let mut got = Vec::new();
        for step in case["steps"].as_array().unwrap() {
            let t = shell(step);
            let next = observe(&t);
            let quiet = t.latest_run_stop_requested == Some(true);
            if let Some(n) =
                gate.observe("t", prior.as_ref(), &next, t.archived_at.is_some(), quiet)
            {
                got.push(match n {
                    Notice::Input => "input",
                    Notice::Completion => "completion",
                });
            }
            prior = Some(next);
        }
        let want: Vec<&str> = case["notices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        if got != want {
            failures.push(format!(
                "gate {}: rust {got:?} != web {want:?}",
                case["name"]
            ));
        }
    }
    let pinned: Vec<ThreadShell> = fixture["pinned"]["shells"]
        .as_array()
        .unwrap()
        .iter()
        .map(shell)
        .collect();
    let mut rows: Vec<(&str, &ThreadShell)> = pinned.iter().map(|t| ("env", t)).collect();
    sort_pinned(&mut rows);
    let got: Vec<&str> = rows.iter().map(|(_, t)| t.id.as_str()).collect();
    let want: Vec<&str> = fixture["pinned"]["order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    if got != want {
        failures.push(format!("pinned order: rust {got:?} != web {want:?}"));
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn iso_parsing_matches_date_parse() {
    assert_eq!(iso_ms("1970-01-01T00:00:00.000Z"), Some(0));
    assert_eq!(iso_ms("1970-01-01T00:00:01.5Z"), Some(1500));
    assert_eq!(iso_ms("1970-01-01T01:00:00+01:00"), Some(0));
    assert_eq!(iso_ms("soon"), None);
}
