//! Desktop notifications when a thread needs the user or finishes, while the window isn't
//! focused (`ThreadNotificationCoordinator` on the web). The gate is t3-client's port of the
//! web's; delivery is `notify-send` (libnotify), whose "Open thread" action opens it here.
use std::collections::{HashMap, HashSet};

use t3_client::shelves::{Notice, NotificationGate, Observation, observe, sidebar_status};

use crate::{App, HANDLE};

/// One environment's gate and what it saw last.
#[derive(Default)]
pub struct Watch {
    gate: NotificationGate,
    seen: HashMap<String, Observation>,
}

impl App {
    /// After a shell update from environment `idx`: notify what changed for the user.
    pub fn watch_threads(&mut self, idx: usize) {
        let mut notices = Vec::new();
        {
            let env = &mut self.envs[idx];
            let watch = &mut env.watch;
            let mut keys = HashSet::new();
            for t in env.shell.threads.iter().filter(|t| t.is_sidebar_row()) {
                keys.insert(t.id.clone());
                let next = observe(t);
                let prior = watch.seen.get(&t.id);
                if prior == Some(&next) {
                    continue;
                }
                let quiet = t.latest_run_stop_requested == Some(true);
                if let Some(n) =
                    watch
                        .gate
                        .observe(&t.id, prior, &next, t.archived_at.is_some(), quiet)
                {
                    notices.push((n, sidebar_status(t), t.id.clone(), t.title.clone()));
                }
                watch.seen.insert(t.id.clone(), next);
            }
            watch.seen.retain(|k, _| keys.contains(k));
            watch.gate.retain(&keys);
        }
        if !self.prefs.notifications || self.window_focused {
            return;
        }
        let env_id = self.envs[idx].cred.environment_id.clone();
        let machine = (self.envs.len() > 1).then(|| self.env_name(idx));
        for (notice, status, thread, title) in notices {
            let heading = match (notice, status) {
                (Notice::Completion, _) => "Thread completed",
                (_, "approval") => "Approval needed",
                (_, "limited") => "Usage limit reached",
                (_, "failed") => "Thread failed",
                (_, "request") => "Guest request",
                _ => "Input needed",
            };
            let body = match &machine {
                Some(m) => format!("{title} · {m}"),
                None => title,
            };
            send(heading, &body, env_id.clone(), thread);
        }
    }
}

/// Shows one notification; clicking "Open thread" opens it in the client.
fn send(heading: &str, body: &str, env_id: String, thread: String) {
    let mut cmd = std::process::Command::new("notify-send");
    cmd.args([
        "--app-name",
        "T3 Code",
        "--action",
        "open=Open thread",
        heading,
        body,
    ])
    // Started from an AppImage (the T3 Code desktop app), this would load its libnotify.
    .env_remove("LD_LIBRARY_PATH")
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null());
    let Ok(child) = cmd.spawn() else { return };
    // The action waits for the notification to close, so it gets its own thread.
    std::thread::spawn(move || {
        let Ok(out) = child.wait_with_output() else {
            return;
        };
        if String::from_utf8_lossy(&out.stdout).trim() != "open" {
            return;
        }
        if let Some(handle) = HANDLE.get().cloned() {
            handle.update(move |s: &mut App, _| {
                if let Some(env) = s.env_index(&env_id) {
                    s.settings_open = false;
                    s.open_thread_in(env, thread);
                }
            });
        }
    });
}
