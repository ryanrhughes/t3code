//! Source control status and the actions the web offers on it: a port of
//! `apps/web/src/components/GitActionsControl.logic.ts` (`resolveQuickAction`, `buildMenuItems`)
//! and `applyGitStatusStreamEvent` from `packages/shared/src/git.ts`, kept equal to them by
//! `tests/git_actions.rs` (fixtures from the real TypeScript).
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VcsStatus {
    pub is_repo: bool,
    pub source_control_provider: Option<Provider>,
    pub has_primary_remote: bool,
    pub is_default_ref: bool,
    pub ref_name: Option<String>,
    pub has_working_tree_changes: bool,
    pub working_tree: WorkingTree,
    pub branch_changes: Option<Totals>,
    pub has_upstream: bool,
    pub ahead_count: u32,
    pub behind_count: u32,
    pub ahead_of_default_count: Option<u32>,
    pub pr: Option<ChangeRequest>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Provider {
    pub kind: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct WorkingTree {
    pub files: Vec<FileStat>,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct FileStat {
    pub path: String,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Totals {
    pub base_ref: Option<String>,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ChangeRequest {
    pub number: u32,
    pub title: String,
    pub url: String,
    pub state: String,
}

/// The two halves `subscribeVcsStatus` sends separately: the working tree, and what the remote
/// knows (upstream, ahead/behind, the open PR).
#[derive(Clone, Debug, Default)]
pub struct StatusStream {
    local: Option<Value>,
    remote: Option<Value>,
}

impl StatusStream {
    /// Folds in one stream event; the merged status, once there is one.
    pub fn apply(&mut self, event: &Value) -> Option<VcsStatus> {
        match event["_tag"].as_str()? {
            "snapshot" => {
                self.local = Some(event["local"].clone());
                self.remote = Some(event["remote"].clone());
            }
            "localUpdated" => self.local = Some(event["local"].clone()),
            "remoteUpdated" => {
                self.remote = Some(event["remote"].clone());
                self.local.get_or_insert_with(|| {
                    json!({
                        "isRepo": true,
                        "hasPrimaryRemote": false,
                        "isDefaultRef": false,
                        "refName": null,
                        "hasWorkingTreeChanges": false,
                        "workingTree": { "files": [], "insertions": 0, "deletions": 0 },
                    })
                });
            }
            _ => return None,
        }
        self.status()
    }

    /// Both halves as one object, as `mergeGitStatusParts`: the remote over the local, or no
    /// upstream while the remote is unknown.
    pub fn merged(&self) -> Option<Value> {
        let mut merged = self.local.clone()?;
        let remote = match &self.remote {
            Some(r) if r.is_object() => r.clone(),
            _ => json!({
                "hasUpstream": false, "aheadCount": 0, "behindCount": 0,
                "aheadOfDefaultCount": 0, "pr": null,
            }),
        };
        if let (Some(m), Some(r)) = (merged.as_object_mut(), remote.as_object()) {
            m.extend(r.clone());
        }
        Some(merged)
    }

    fn status(&self) -> Option<VcsStatus> {
        serde_json::from_value(self.merged()?).ok()
    }
}

/// What a host calls a pull request, short ("PR") and spelled out ("pull request"), as its
/// source control definition does. No host reads as GitHub, the registry's first; one the
/// client doesn't know gets the generic words.
pub fn change_request_terms(status: &VcsStatus) -> (&'static str, &'static str) {
    match status
        .source_control_provider
        .as_ref()
        .map(|p| p.kind.as_str())
    {
        None | Some("github" | "gitcafe" | "forgejo" | "bitbucket" | "azure-devops") => {
            ("PR", "pull request")
        }
        Some("gitlab") => ("MR", "merge request"),
        Some(_) => ("change request", "change request"),
    }
}

/// The short name alone: "PR", "MR".
pub fn change_request_label(status: &VcsStatus) -> &'static str {
    change_request_terms(status).0
}

#[derive(Clone, Debug, PartialEq)]
pub enum QuickKind {
    /// Run `git.runStackedAction` with this action.
    Run(&'static str),
    Pull,
    Publish,
    /// Nothing to run; the hint says why.
    Hint(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuickAction {
    pub label: String,
    pub kind: QuickKind,
}

impl QuickAction {
    pub fn disabled(&self) -> bool {
        matches!(self.kind, QuickKind::Hint(_))
    }
}

fn hint(label: &str, hint: impl Into<String>) -> QuickAction {
    QuickAction {
        label: label.into(),
        kind: QuickKind::Hint(hint.into()),
    }
}

fn run(label: impl Into<String>, action: &'static str) -> QuickAction {
    QuickAction {
        label: label.into(),
        kind: QuickKind::Run(action),
    }
}

/// The one action the git button runs, as `resolveQuickAction`.
pub fn quick_action(
    status: Option<&VcsStatus>,
    busy: bool,
    is_default_ref: bool,
    has_primary_remote: bool,
) -> QuickAction {
    if busy {
        return hint("Commit", "Git action in progress.");
    }
    let Some(s) = status else {
        return hint("Commit", "Git status is unavailable.");
    };
    let (pr, singular) = change_request_terms(s);
    let open_pr = s.pr.as_ref().is_some_and(|p| p.state == "open");
    let ahead = s.ahead_count > 0;
    let behind = s.behind_count > 0;
    let default_delta = s.ahead_of_default_count.unwrap_or(s.ahead_count) > 0;
    let up_to_date = || hint("Commit", "Branch is up to date. No action needed.");
    let push = || {
        if open_pr || is_default_ref {
            run(
                "Push",
                if is_default_ref {
                    "commit_push"
                } else {
                    "push"
                },
            )
        } else {
            run(format!("Push & create {pr}"), "create_pr")
        }
    };

    if s.ref_name.is_none() {
        return hint(
            "Commit",
            format!("Create and checkout a ref before pushing or opening a {singular}."),
        );
    }
    if s.has_working_tree_changes {
        if behind || (!s.has_upstream && !has_primary_remote) {
            return run("Commit", "commit");
        }
        if open_pr || is_default_ref {
            return run("Commit & push", "commit_push");
        }
        return run(format!("Commit, push & {pr}"), "commit_push_pr");
    }
    if !s.has_upstream {
        if !has_primary_remote {
            return QuickAction {
                label: "Publish repository".into(),
                kind: QuickKind::Publish,
            };
        }
        if !ahead {
            return if open_pr {
                up_to_date()
            } else {
                hint("Push", "No local commits to push.")
            };
        }
        return push();
    }
    if ahead && behind {
        return hint(
            "Sync ref",
            "Branch has diverged from upstream. Rebase/merge first.",
        );
    }
    if behind {
        return QuickAction {
            label: "Pull".into(),
            kind: QuickKind::Pull,
        };
    }
    if ahead {
        return push();
    }
    if open_pr && s.has_upstream {
        return up_to_date();
    }
    if default_delta && !is_default_ref {
        return run(format!("Create {pr}"), "create_pr");
    }
    up_to_date()
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    /// "commit", "push" or "pr".
    pub id: &'static str,
    pub label: String,
    pub disabled: bool,
    /// What it runs.
    pub action: &'static str,
}

/// The git menu's items, as `buildMenuItems`.
pub fn menu_items(
    status: Option<&VcsStatus>,
    busy: bool,
    has_primary_remote: bool,
) -> Vec<MenuItem> {
    let Some(s) = status else {
        return Vec::new();
    };
    let has_branch = s.ref_name.is_some();
    let open_pr = s.pr.as_ref().is_some_and(|p| p.state == "open");
    let behind = s.behind_count > 0;
    let default_delta = s.ahead_of_default_count.unwrap_or(s.ahead_count) > 0;
    let can_reach = s.has_upstream || (has_primary_remote && !s.has_upstream);
    let commit = MenuItem {
        id: "commit",
        label: "Commit".into(),
        disabled: busy || !s.has_working_tree_changes,
        action: "commit",
    };
    if !has_primary_remote {
        return vec![commit];
    }
    let push = MenuItem {
        id: "push",
        label: "Push".into(),
        disabled: !(!busy && has_branch && !behind && s.ahead_count > 0 && can_reach),
        action: "push",
    };
    if open_pr {
        return vec![commit, push];
    }
    let pr = MenuItem {
        id: "pr",
        label: format!("Create {}", change_request_label(s)),
        disabled: !(!busy && has_branch && default_delta && !behind && can_reach),
        action: "create_pr",
    };
    vec![commit, push, pr]
}

/// Whether an action on the default branch asks first, as `requiresDefaultBranchConfirmation`.
pub fn confirm_on_default_ref(action: &str, is_default_ref: bool) -> bool {
    is_default_ref
        && matches!(
            action,
            "push" | "create_pr" | "commit_push" | "commit_push_pr"
        )
}

/// The +/− totals the git button shows, as `resolveChangesTotals`: the branch's against its
/// base when it has any, else the uncommitted ones. The ref is `None` for uncommitted.
pub fn changes_totals(s: &VcsStatus) -> (u32, u32, Option<String>) {
    let tree = (s.working_tree.insertions, s.working_tree.deletions, None);
    match &s.branch_changes {
        Some(b) if b.insertions > 0 || b.deletions > 0 => {
            (b.insertions, b.deletions, b.base_ref.clone())
        }
        _ if tree.0 > 0 || tree.1 > 0 => tree,
        Some(b) => (b.insertions, b.deletions, b.base_ref.clone()),
        None => tree,
    }
}
