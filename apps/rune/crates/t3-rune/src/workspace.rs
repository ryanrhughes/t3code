//! Where a new thread works: the project's checkout ("Local") or a new git worktree off a base
//! branch, as the web's env-mode selector and base branch picker set `workspaceStrategy`. The
//! default follows the server's `defaultThreadEnvMode`.
use rune::{El, text};
use rune_kit::{Button, ButtonVariant, ControlSize};
use serde_json::{Value, json};

use crate::theme::Look;
use crate::{App, HANDLE, Popup};

/// A project's local branches, for the base branch picker. Keyed by machine and root: two
/// machines can have the same path.
pub struct Refs {
    pub env_id: String,
    pub root: String,
    pub loading: bool,
    /// When a read failed; another try waits a few seconds.
    pub failed_at: Option<i64>,
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

    /// The draft's machine and project root, which a branch list and a picked base belong to.
    fn draft_key(&self) -> Option<(String, String)> {
        Some((self.env()?.cred.environment_id.clone(), self.draft_root()?))
    }

    /// The branch a new worktree starts from: the pick for this project, else the default
    /// branch, else the one checked out.
    pub fn draft_base(&self) -> Option<String> {
        let key = self.draft_key()?;
        if let Some((env, root, branch)) = &self.draft_base {
            if (env, root) == (&key.0, &key.1) {
                return Some(branch.clone());
            }
        }
        let refs = self
            .refs
            .as_ref()
            .filter(|r| (&r.env_id, &r.root) == (&key.0, &key.1))?;
        refs.list
            .iter()
            .find(|r| r.1)
            .or_else(|| refs.list.iter().find(|r| r.2))
            .map(|r| r.0.clone())
    }

    /// Loads the draft project's local branches, once per project.
    pub fn load_refs(&mut self) {
        let Some((env_id, root)) = self.draft_key() else {
            return;
        };
        let now = crate::now_secs();
        if self.refs.as_ref().is_some_and(|r| {
            (&r.env_id, &r.root) == (&env_id, &root) && r.failed_at.is_none_or(|at| now - at < 10)
        }) {
            return;
        }
        let (Some(session), Some(handle)) = (self.session(), HANDLE.get().cloned()) else {
            return;
        };
        self.refs = Some(Refs {
            env_id: env_id.clone(),
            root: root.clone(),
            loading: true,
            failed_at: None,
            list: Vec::new(),
        });
        let input = json!({ "cwd": root, "refKind": "local", "limit": 100 });
        session.call("vcs.listRefs", input, move |result| {
            handle.update(move |s: &mut App, tx| {
                let Some(refs) = s
                    .refs
                    .as_mut()
                    .filter(|r| (&r.env_id, &r.root) == (&env_id, &root))
                else {
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
                    Err(e) => {
                        // Tried again shortly, from poll; until then the draft can't launch.
                        refs.failed_at = Some(crate::now_secs());
                        s.toast(format!("Couldn't list branches: {e}"), tx);
                    }
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
                        let branch = label.strip_prefix("from ").map(str::to_owned);
                        s.draft_base = s.draft_key().zip(branch).map(|((e, r), b)| (e, r, b));
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
