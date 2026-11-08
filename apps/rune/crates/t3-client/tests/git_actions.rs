//! Replays tests/fixtures/git-actions.json, produced by the web's own `resolveQuickAction`,
//! `buildMenuItems` and `requiresDefaultBranchConfirmation` over a grid of branch states
//! (`node apps/rune/scripts/git-actions-reference.ts`).
use serde_json::{Value, json};
use t3_client::git::{
    QuickKind, StatusStream, VcsStatus, confirm_on_default_ref, menu_items, quick_action,
};

#[test]
fn git_actions_match_the_web() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/git-actions.json")).unwrap();
    let statuses: Vec<VcsStatus> = fixture["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| serde_json::from_value(s.clone()).unwrap())
        .collect();
    let mut failures = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let status = &statuses[case["status"].as_u64().unwrap() as usize];
        let busy = case["busy"] == true;
        let (default_ref, remote) = (
            case["isDefaultRef"] == true,
            case["hasPrimaryRemote"] == true,
        );
        let q = quick_action(Some(status), busy, default_ref, remote);
        let (kind, detail) = match &q.kind {
            QuickKind::Run(a) => ("run_action", json!(a)),
            QuickKind::Pull => ("run_pull", Value::Null),
            QuickKind::Publish => ("open_publish", Value::Null),
            QuickKind::Hint(h) => ("show_hint", json!(h)),
        };
        let got = json!([q.label, q.disabled(), kind, detail]);
        let result = |k: &str| &fixture["results"][case[k].as_u64().unwrap() as usize];
        if got != *result("quick") {
            failures.push(format!("quick {case}: rust {got}"));
        }
        if !case["menu"].is_null() {
            let got: Value = menu_items(Some(status), busy, remote)
                .iter()
                .map(|m| json!([m.id, m.label, m.disabled, m.action]))
                .collect();
            if got != *result("menu") {
                failures.push(format!("menu {case}: rust {got}"));
            }
        }
    }
    for c in fixture["confirm"].as_array().unwrap() {
        let got = confirm_on_default_ref(c[0].as_str().unwrap(), c[1] == true);
        if got != c[2] {
            failures.push(format!("confirm {c}: rust {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of the web's cases differ:\n{}",
        failures.len(),
        failures
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn status_halves_merge_like_the_web() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/git-actions.json")).unwrap();
    let mut stream = StatusStream::default();
    for (i, step) in fixture["stream"].as_array().unwrap().iter().enumerate() {
        assert!(stream.apply(&step["event"]).is_some(), "step {i} decodes");
        assert_eq!(stream.merged().as_ref(), Some(&step["merged"]), "step {i}");
    }
}
