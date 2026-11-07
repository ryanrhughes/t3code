//! Where a new thread works: the project's checkout ("Local") or a new git worktree off a base
//! branch, as the web's env-mode selector and base branch picker set `workspaceStrategy`. The
//! default follows the server's `defaultThreadEnvMode`.
use rune::{El, text};
use rune_kit::{Button, ButtonVariant, ControlSize};
use serde_json::{Value, json};

use crate::theme::Look;
use crate::{App, HANDLE, Popup};

/// A project's local branches, for the base branch picker.
pub struct Refs {
    pub root: String,
    pub loading: bool,
    /// (name, is the default branch, is checked out).
    pub list: Vec<(String, bool, bool)>,
}

impl App {
    fn draft_root(&self) -> Option<String> {
        let project = self.draft_project.as_deref()?;
        Some(self.shell().project(project)?.workspace_root.clone())
    }

    /// Whether the draft starts in a new worktree: the user's pick, else the server default.
    pub fn draft_in_worktree(&self) -> bool {
        self.draft_worktree.unwrap_or_else(|| {
            self.config()
                .and_then(|c| {
                    c.settings
                        .get("defaultThreadEnvMode")?
                        .as_str()
                        .map(str::to_owned)
                })
                .is_some_and(|m| m == "worktree")
        })
    }

    pub fn toggle_draft_worktree(&mut self) {
        let next = !self.draft_in_worktree();
        self.draft_worktree = Some(next);
        if next {
            self.load_refs();
        }
    }

    /// The branch a new worktree starts from: the pick, else the default branch, else the one
    /// checked out.
    pub fn draft_base(&self) -> Option<String> {
        if let Some(b) = &self.draft_base {
            return Some(b.clone());
        }
        let refs = self.refs.as_ref()?;
        refs.list
            .iter()
            .find(|r| r.1)
            .or_else(|| refs.list.iter().find(|r| r.2))
            .map(|r| r.0.clone())
    }

    /// Loads the draft project's local branches, once per project.
    pub fn load_refs(&mut self) {
        let Some(root) = self.draft_root() else {
            return;
        };
        if self.refs.as_ref().is_some_and(|r| r.root == root) {
            return;
        }
        let (Some(session), Some(handle)) = (self.session(), HANDLE.get().cloned()) else {
            return;
        };
        self.refs = Some(Refs {
            root: root.clone(),
            loading: true,
            list: Vec::new(),
        });
        let input = json!({ "cwd": root, "refKind": "local", "limit": 100 });
        session.call("vcs.listRefs", input, move |result| {
            handle.update(move |s: &mut App, tx| {
                let Some(refs) = s.refs.as_mut().filter(|r| r.root == root) else {
                    return;
                };
                refs.loading = false;
                match result {
                    Ok(v) => {
                        refs.list = v["refs"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|r| {
                                Some((
                                    r["name"].as_str()?.to_owned(),
                                    r["isDefault"] == true,
                                    r["current"] == true,
                                ))
                            })
                            .collect();
                        if v["isRepo"] == false {
                            s.toast(
                                "This project isn't a git repository, so it can't use worktrees",
                                tx,
                            );
                            s.draft_worktree = Some(false);
                        }
                    }
                    Err(e) => s.toast(format!("Couldn't list branches: {e}"), tx),
                }
            });
        });
    }

    /// The `workspaceStrategy` a new thread launches with.
    pub fn draft_workspace(&self) -> Value {
        match (self.draft_in_worktree(), self.draft_base()) {
            (true, Some(base)) => json!({ "type": "worktree", "baseRef": base }),
            _ => json!({ "type": "root" }),
        }
    }
}

/// The draft's workspace controls: Local / New worktree, and the base branch for a worktree.
pub fn controls(s: &App, look: &Look, layer: &mut Vec<El>) -> Vec<El> {
    let k = &look.kit;
    let worktree = s.draft_in_worktree();
    let mut out = vec![
        Button::new(if worktree { "New worktree" } else { "Local" })
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .help("Where the thread works: this checkout, or a new git worktree")
            .on_press(|s: &mut App| s.toggle_draft_worktree())
            .build(k)
            .key("workspace-toggle"),
    ];
    if !worktree {
        return out;
    }
    match s.refs.as_ref() {
        Some(r) if r.loading => {
            out.push(text("Loading branches…").font(k.ty.small).color(look.muted))
        }
        Some(r) if !r.list.is_empty() => {
            let labels: Vec<String> = r.list.iter().map(|b| format!("from {}", b.0)).collect();
            let current = s.draft_base().map(|b| format!("from {b}"));
            out.push(
                rune_kit::Select::new("base-branch", &labels)
                    .current(current.as_deref())
                    .open(s.popup == Some(Popup::BaseBranch))
                    .on_toggle(|s: &mut App| {
                        s.popup = (s.popup != Some(Popup::BaseBranch)).then_some(Popup::BaseBranch)
                    })
                    .on_close(|s: &mut App| s.popup = None)
                    .on_choose(|s: &mut App, label: &str, _: &mut rune::Tx| {
                        s.popup = None;
                        s.draft_base = label.strip_prefix("from ").map(str::to_owned);
                    })
                    .build(k, layer)
                    .key("base-branch-select"),
            );
        }
        _ => {}
    }
    out
}

/// The open thread's branch, and whether it has its own worktree.
pub fn thread_branch(s: &App, look: &Look) -> Option<El> {
    let t = s.selected.as_deref().and_then(|id| s.shell().thread(id))?;
    let branch = t.branch.clone()?;
    let label = if t.worktree_path.is_some() {
        format!("{branch} · worktree")
    } else {
        branch
    };
    Some(
        text(label)
            .font(look.kit.ty.small)
            .color(look.muted)
            .truncate()
            .key("thread-branch"),
    )
}
