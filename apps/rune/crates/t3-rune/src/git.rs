//! Source control for the open thread, as the web's `GitActionsControl`: a header button that
//! runs the next sensible action (commit, push, open a PR), a menu with each step on its own,
//! the commit dialog, and the default-branch check. The server does the git work in
//! `git.runStackedAction` and writes the commit message when the user leaves it empty.
use std::collections::HashSet;

use rune::{El, TextEdit, Tx, col, row, text};
use rune_kit::{
    Button, ButtonVariant, Checkbox, ControlSize, Dialog, IconButton, IconButtonVariant, Menu,
    MenuItem, TextArea,
};
use serde_json::{Value, json};
use t3_client::git::{self as rules, QuickKind, StatusStream, VcsStatus};
use t3_client::protocol::{self, thread_command};

use crate::theme::Look;
use crate::{App, HANDLE, Popup};

/// The open thread's repository: which one and what it looks like.
pub struct Git {
    pub env_id: String,
    pub cwd: String,
    /// The session the status subscription runs on; a re-pair replaces it.
    generation: u64,
    stream: StatusStream,
    pub status: Option<VcsStatus>,
}

/// An action running on a checkout, kept apart from `Git` so leaving the thread and coming
/// back still finds it busy. Only the action that set it clears it.
pub struct Running {
    id: String,
    pub label: String,
    /// The newest line a hook printed.
    pub output: Option<String>,
    phase: Option<String>,
}

/// One `git.runStackedAction`, before it runs.
#[derive(Clone)]
pub struct GitRun {
    pub action: &'static str,
    pub message: Option<String>,
    pub files: Option<Vec<String>>,
    pub feature_branch: bool,
}

pub struct CommitDialog {
    pub message: TextEdit,
    pub excluded: HashSet<String>,
}

fn status_key(env: &str, cwd: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("vcs-status", env, cwd).hash(&mut h);
    h.finish()
}

impl App {
    /// Follows the open thread's checkout with a status subscription, swapping it when the
    /// thread (or the machine) changes.
    pub fn sync_git(&mut self) {
        let want = self.selected.as_ref().and(self.workspace_cwd()).zip(
            self.env()
                .map(|e| (e.cred.environment_id.clone(), e.generation)),
        );
        let have = self
            .git
            .as_ref()
            .map(|g| (g.cwd.clone(), (g.env_id.clone(), g.generation)));
        if want == have {
            return;
        }
        if let Some(old) = self.git.take() {
            if let Some(i) = self.env_index(&old.env_id) {
                if let Some(session) = &self.envs[i].session {
                    session.unsubscribe(status_key(&old.env_id, &old.cwd));
                }
            }
        }
        self.commit_dialog = None;
        self.git_confirm = None;
        let (Some((cwd, (env_id, generation))), Some(session), Some(handle)) =
            (want, self.session(), HANDLE.get().cloned())
        else {
            return;
        };
        let (e, c) = (env_id.clone(), cwd.clone());
        session.subscribe(
            status_key(&env_id, &cwd),
            "subscribeVcsStatus",
            json!({ "cwd": cwd }),
            move |frame| {
                let (e, c) = (e.clone(), c.clone());
                handle.update(move |s: &mut App, _| {
                    let Some(g) = s
                        .git
                        .as_mut()
                        .filter(|g| (&g.env_id, &g.cwd, g.generation) == (&e, &c, generation))
                    else {
                        return;
                    };
                    if let Some(Ok(event)) = frame {
                        if let Some(status) = g.stream.apply(&event) {
                            g.status = Some(status);
                        }
                    }
                });
            },
        );
        self.git = Some(Git {
            env_id,
            cwd,
            generation,
            stream: StatusStream::default(),
            status: None,
        });
    }

    /// Asks the server to re-read the checkout now, as the web does when its git menu opens;
    /// the answer arrives on the status stream.
    pub fn refresh_git(&mut self) {
        if let (Some(g), Some(session)) = (self.git.as_ref(), self.session()) {
            session.call("vcs.refreshStatus", json!({ "cwd": g.cwd }), |_| {});
        }
    }

    fn git_status(&self) -> Option<&VcsStatus> {
        self.git.as_ref()?.status.as_ref().filter(|s| s.is_repo)
    }

    fn git_key(&self) -> Option<(String, String)> {
        let g = self.git.as_ref()?;
        Some((g.env_id.clone(), g.cwd.clone()))
    }

    fn git_running(&self) -> Option<&Running> {
        self.git_running.get(&self.git_key()?)
    }

    fn git_busy(&self) -> bool {
        self.git_running().is_some()
    }

    /// The running action on `key`, if it's still the one with `id`.
    fn running_mut(&mut self, key: &(String, String), id: &str) -> Option<&mut Running> {
        self.git_running.get_mut(key).filter(|r| r.id == id)
    }

    fn end_running(&mut self, key: &(String, String), id: &str) {
        if self.running_mut(key, id).is_some() {
            self.git_running.remove(key);
        }
    }

    /// The header button: runs its action, asking first on the default branch.
    pub fn run_quick_git(&mut self, tx: &mut Tx) {
        let status = self.git_status();
        let q = rules::quick_action(
            status,
            self.git_busy(),
            status.is_some_and(|s| s.is_default_ref),
            status.is_some_and(|s| s.has_primary_remote),
        );
        match q.kind {
            QuickKind::Run(action) => self.request_git(
                GitRun {
                    action,
                    message: None,
                    files: None,
                    feature_branch: false,
                },
                tx,
            ),
            QuickKind::Pull => self.pull(tx),
            QuickKind::Publish => self.toast("Publish the repository from the web app", tx),
            QuickKind::Hint(h) => self.toast(h, tx),
        }
    }

    pub fn open_commit_dialog(&mut self) {
        self.popup = None;
        self.commit_dialog = Some(CommitDialog {
            message: crate::focused(""),
            excluded: HashSet::new(),
        });
    }

    pub fn commit_from_dialog(&mut self, feature_branch: bool, tx: &mut Tx) {
        let Some(d) = self.commit_dialog.take() else {
            return;
        };
        let files: Vec<String> = self
            .git_status()
            .map(|s| {
                s.working_tree
                    .files
                    .iter()
                    .map(|f| f.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let picked: Vec<String> = files
            .iter()
            .filter(|f| !d.excluded.contains(*f))
            .cloned()
            .collect();
        if picked.is_empty() {
            return self.toast("Pick at least one file to commit", tx);
        }
        let message = d.message.text.trim();
        self.request_git(
            GitRun {
                action: "commit",
                message: (!message.is_empty()).then(|| message.to_owned()),
                files: (picked.len() < files.len()).then_some(picked),
                feature_branch,
            },
            tx,
        );
    }

    /// Runs an action, or asks first when it would push to the default branch.
    pub fn request_git(&mut self, run: GitRun, tx: &mut Tx) {
        self.popup = None;
        let default_ref = self.git_status().is_some_and(|s| s.is_default_ref);
        if !run.feature_branch && rules::confirm_on_default_ref(run.action, default_ref) {
            self.git_confirm = Some(run);
            return;
        }
        self.run_git(run, tx);
    }

    /// The default-branch question's answer: run as asked, or on a new feature branch.
    pub fn confirm_git(&mut self, feature_branch: bool, tx: &mut Tx) {
        if let Some(mut run) = self.git_confirm.take() {
            run.feature_branch = feature_branch;
            self.run_git(run, tx);
        }
    }

    fn run_git(&mut self, run: GitRun, tx: &mut Tx) {
        if self.git_busy() {
            return self.toast("A git action is already running", tx);
        }
        let (Some(session), Some(handle)) = (self.session(), HANDLE.get().cloned()) else {
            return;
        };
        let (Some(key), Some(thread)) = (self.git_key(), self.selected.clone()) else {
            return;
        };
        let id = protocol::new_id();
        let mut input =
            json!({ "actionId": id, "cwd": key.1, "action": run.action, "threadId": thread });
        if let Some(m) = &run.message {
            input["commitMessage"] = json!(m);
        }
        if let Some(f) = &run.files {
            input["filePaths"] = json!(f);
        }
        if run.feature_branch {
            input["featureBranch"] = json!(true);
        }
        self.git_running.insert(
            key.clone(),
            Running {
                id: id.clone(),
                label: "Starting…".into(),
                output: None,
                phase: None,
            },
        );
        session.stream("git.runStackedAction", input, move |frame| {
            let (key, id, thread) = (key.clone(), id.clone(), thread.clone());
            handle.update(move |s: &mut App, tx| s.on_git_progress(&key, &id, &thread, frame, tx));
        });
    }

    /// One frame of an action's stream. It stays busy until the stream ends: the server still
    /// refreshes status after `action_finished`.
    fn on_git_progress(
        &mut self,
        key: &(String, String),
        id: &str,
        thread: &str,
        frame: Option<Result<Value, String>>,
        tx: &mut Tx,
    ) {
        let ev = match frame {
            Some(Ok(ev)) => ev,
            Some(Err(e)) => {
                self.end_running(key, id);
                return self.toast(format!("Git action failed: {e}"), tx);
            }
            None => return self.end_running(key, id),
        };
        let str_of = |k: &str| ev[k].as_str().map(str::to_owned);
        match ev["kind"].as_str().unwrap_or("") {
            "phase_started" => {
                if let Some(r) = self.running_mut(key, id) {
                    r.phase = str_of("label");
                    r.label = r.phase.clone().unwrap_or_default();
                }
            }
            "hook_started" => {
                if let Some(r) = self.running_mut(key, id) {
                    r.label = format!("Running {}…", str_of("hookName").unwrap_or_default());
                }
            }
            "hook_output" => {
                if let Some(r) = self.running_mut(key, id) {
                    r.output = str_of("text").and_then(|t| t.lines().last().map(str::to_owned));
                }
            }
            "hook_finished" => {
                if let Some(r) = self.running_mut(key, id) {
                    r.label = r.phase.clone().unwrap_or_default();
                    r.output = None;
                }
            }
            "action_failed" => {
                self.end_running(key, id);
                self.toast(
                    format!(
                        "Git action failed: {}",
                        str_of("message").unwrap_or_default()
                    ),
                    tx,
                );
            }
            "action_finished" => {
                if let Some(r) = self.running_mut(key, id) {
                    r.label = "Finishing…".into();
                    r.output = None;
                }
                let r = &ev["result"];
                // A new feature branch: the thread follows it, as the web's metadata patch.
                if let Some(branch) = (r["branch"]["status"] == "created")
                    .then(|| r["branch"]["name"].as_str())
                    .flatten()
                {
                    if let Some(i) = self.env_index(&key.0) {
                        if let Some(session) = &self.envs[i].session {
                            session.dispatch(thread_command(
                                "thread.metadata.update",
                                thread,
                                json!({ "branch": branch }),
                            ));
                        }
                    }
                }
                let title = if r["pr"]["status"] == "created" {
                    let label = self.git_status().map_or("PR", rules::change_request_label);
                    format!("Created {label}")
                } else {
                    r["toast"]["title"].as_str().unwrap_or("Done").to_owned()
                };
                let msg = match r["toast"]["description"].as_str() {
                    Some(d) if r["pr"]["status"] != "created" => format!("{title}: {d}"),
                    _ => title,
                };
                self.toast(msg, tx);
                if let Some(url) = r["pr"]["url"]
                    .as_str()
                    .filter(|_| r["pr"]["status"] == "created")
                {
                    let _ = tx.open_url(url);
                }
            }
            _ => {}
        }
    }

    fn pull(&mut self, tx: &mut Tx) {
        let (Some(session), Some(handle)) = (self.session(), HANDLE.get().cloned()) else {
            return;
        };
        let Some(key) = self.git_key().filter(|_| !self.git_busy()) else {
            return;
        };
        let id = protocol::new_id();
        self.git_running.insert(
            key.clone(),
            Running {
                id: id.clone(),
                label: "Pulling latest changes…".into(),
                output: None,
                phase: None,
            },
        );
        session.call("vcs.pull", json!({ "cwd": key.1 }), move |result| {
            handle.update(move |s: &mut App, tx| {
                s.end_running(&key, &id);
                match result {
                    Ok(v) if v["status"] == "skipped_up_to_date" => {
                        s.toast("Already up to date", tx)
                    }
                    Ok(v) => s.toast(
                        format!("Pulled {}", v["refName"].as_str().unwrap_or("the branch")),
                        tx,
                    ),
                    Err(e) => s.toast(format!("Couldn't pull: {e}"), tx),
                }
            });
        });
        let _ = tx;
    }
}

static GIT: rune::Icon = rune::Icon {
    name: "t3-git",
    view: 16.0,
    body: r#"<circle cx="5" cy="4" r="1.6" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="5" cy="12" r="1.6" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="11" cy="6" r="1.6" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M5 5.6V10.4M11 7.6C11 9.5 9 10 6.4 11" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#,
};

/// The header's git control: the next action, and a menu with each step.
pub fn control(s: &App, look: &Look, layer: &mut Vec<El>) -> Option<El> {
    let status = s.git_status()?;
    let k = &look.kit;
    let busy = s.git_busy();
    let q = rules::quick_action(
        Some(status),
        busy,
        status.is_default_ref,
        status.has_primary_remote,
    );
    let running = s.git_running();
    let label = running.map_or(q.label.clone(), |r| r.label.clone());
    let mut quick = Button::new(label)
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .icon(&GIT)
        .disabled(busy)
        .on_press_tx(|s: &mut App, tx| s.run_quick_git(tx));
    if let Some(help) = running.and_then(|r| r.output.clone()) {
        quick = quick.help(help);
    } else if let QuickKind::Hint(h) = &q.kind {
        quick = quick.help(h.clone());
    }
    let open = s.popup == Some(Popup::Git);
    let trigger = IconButton::new(&rune_kit::glyphs::CHEVRON_DOWN)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label("Git actions")
        .expanded(open)
        .on_press(|s: &mut App| {
            s.popup = (s.popup != Some(Popup::Git)).then_some(Popup::Git);
            if s.popup.is_some() {
                s.refresh_git();
            }
        })
        .build(k)
        .key("git-menu-trigger");
    let mut menu = Menu::new("git-menu")
        .trigger(trigger)
        .open(open)
        .min_width(200.0)
        .on_close(|s: &mut App| s.popup = None);
    for item in rules::menu_items(Some(status), busy, status.has_primary_remote) {
        let action = item.action;
        let entry = MenuItem::new(item.label).disabled(item.disabled);
        menu = menu.item(if action == "commit" {
            entry.on_select(|s: &mut App| s.open_commit_dialog())
        } else {
            entry.on_select_tx(move |s: &mut App, tx| {
                s.request_git(
                    GitRun {
                        action,
                        message: None,
                        files: None,
                        feature_branch: false,
                    },
                    tx,
                )
            })
        });
    }
    if status.behind_count > 0 && status.has_upstream {
        menu = menu.item(
            MenuItem::new("Pull")
                .disabled(busy)
                .on_select_tx(|s: &mut App, tx| s.pull(tx)),
        );
    }
    if let Some(pr) = status.pr.as_ref().filter(|p| !p.url.is_empty()) {
        let url = pr.url.clone();
        let label = format!(
            "Open {} #{}",
            rules::change_request_label(status),
            pr.number
        );
        menu = menu
            .separator()
            .item(MenuItem::new(label).on_select_tx(move |s: &mut App, tx| {
                s.popup = None;
                let _ = tx.open_url(&url);
            }));
    }
    Some(
        row()
            .key("git-control")
            .gap(2.0)
            .center_y()
            .shrink()
            .min_w(0.0)
            .child(quick.build(k).key("git-quick").shrink())
            .child(menu.build(k, layer)),
    )
}

/// The commit dialog: an optional message (empty asks the server to write one) and the files.
pub fn commit_dialog(s: &App, look: &Look, layer: &mut Vec<El>) {
    let (Some(d), Some(status)) = (s.commit_dialog.as_ref(), s.git_status()) else {
        return;
    };
    let k = &look.kit;
    let branch = status.ref_name.clone().unwrap_or_default();
    let message = TextArea::new(&d.message)
        .label("Commit message")
        .placeholder("Leave empty to generate one")
        .width(None)
        .auto_grow(3, 8)
        .on_edit(|s: &mut App| match s.commit_dialog.as_mut() {
            Some(d) => &mut d.message,
            None => &mut s.composer,
        })
        .build(k)
        .key("commit-message");
    let mut files = col().w_full().gap(2.0);
    for f in &status.working_tree.files {
        let path = f.path.clone();
        files = files.child(
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .child(
                    Checkbox::new(!d.excluded.contains(&f.path))
                        .label(f.path.clone())
                        .on_toggle(move |s: &mut App| {
                            if let Some(d) = s.commit_dialog.as_mut() {
                                if !d.excluded.remove(&path) {
                                    d.excluded.insert(path.clone());
                                }
                            }
                        })
                        .build(k)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0),
                )
                .child(
                    text(format!("+{} −{}", f.insertions, f.deletions))
                        .font(k.ty.small)
                        .color(look.muted),
                ),
        );
    }
    let buttons = row()
        .w_full()
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Commit on new branch")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press_tx(|s: &mut App, tx| s.commit_from_dialog(true, tx))
                .build(k)
                .key("commit-new-branch"),
        )
        .child(
            Button::new("Commit")
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::Small)
                .on_press_tx(|s: &mut App, tx| s.commit_from_dialog(false, tx))
                .build(k)
                .key("commit-run"),
        );
    Dialog::new("commit-dialog", "Commit changes")
        .description(format!("On {branch}"))
        .open(true)
        .fit()
        .width(520.0)
        .on_close(|s: &mut App| s.commit_dialog = None)
        .child(
            body()
                .child(message)
                .child(
                    col()
                        .w_full()
                        .max_h(240.0)
                        .scroll(rune::ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(files),
                )
                .child(buttons),
        )
        .build(k, layer);
}

/// "Push to default ref?", as `resolveDefaultBranchActionDialogCopy`.
pub fn confirm_dialog(s: &App, look: &Look, layer: &mut Vec<El>) {
    let (Some(run), Some(status)) = (s.git_confirm.as_ref(), s.git_status()) else {
        return;
    };
    let k = &look.kit;
    let branch = status.ref_name.clone().unwrap_or_default();
    let (pr, singular) = rules::change_request_terms(status);
    let includes_commit = matches!(run.action, "commit" | "commit_push" | "commit_push_pr")
        && status.has_working_tree_changes;
    let suffix = format!(
        " on \"{branch}\". You can continue on this ref or create a feature ref and run the same action there."
    );
    let (title, body, go) = match (
        matches!(run.action, "push" | "commit_push"),
        includes_commit,
    ) {
        (true, true) => (
            "Commit & push to default ref?".to_owned(),
            format!("This action will commit and push changes{suffix}"),
            format!("Commit & push to {branch}"),
        ),
        (true, false) => (
            "Push to default ref?".to_owned(),
            format!("This action will push local commits{suffix}"),
            format!("Push to {branch}"),
        ),
        (false, true) => (
            format!("Commit, push & create {pr} from default ref?"),
            format!("This action will commit, push, and create a {singular}{suffix}"),
            format!("Commit, push & create {pr}"),
        ),
        (false, false) => (
            format!("Push & create {pr} from default ref?"),
            format!("This action will push local commits and create a {singular}{suffix}"),
            format!("Push & create {pr}"),
        ),
    };
    let mut feature = Button::new("Check out feature branch & continue")
        .variant(ButtonVariant::Contrast)
        .control_size(ControlSize::Small)
        .on_press_tx(|s: &mut App, tx| s.confirm_git(true, tx));
    // The server names a feature branch from the uncommitted changes.
    if !status.has_working_tree_changes {
        feature = feature.disabled(true).help(format!(
            "There are no changes to commit, so a feature branch can't be named. Create a branch yourself or continue on {branch}."
        ));
    }
    let buttons = confirm_buttons(feature.build(k).key("git-confirm-feature"), go, k);
    Dialog::new("git-confirm", title)
        .open(true)
        .fit()
        .on_close(|s: &mut App| s.git_confirm = None)
        .child(
            self::body()
                .child(text(body).font(k.ty.ui).color(look.muted).wrap().w_full())
                .child(buttons),
        )
        .build(k, layer);
}

fn confirm_buttons(feature: El, go: String, k: &rune_kit::Kit) -> El {
    row()
        .w_full()
        .gap(8.0)
        .justify_end()
        .flex_wrap()
        .child(
            Button::new("Abort")
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.git_confirm = None)
                .build(k)
                .key("git-confirm-abort"),
        )
        .child(
            Button::new(go)
                .variant(ButtonVariant::Neutral)
                .control_size(ControlSize::Small)
                .on_press_tx(|s: &mut App, tx| s.confirm_git(false, tx))
                .build(k)
                .key("git-confirm-continue"),
        )
        .child(feature)
}

/// A dialog's content, inset from its edges.
fn body() -> El {
    col().w_full().gap(12.0).px(16.0).pb(16.0)
}
