//! Settings: this device's preferences, kept in prefs.json, and the server's, read from the
//! config stream and changed with `server.updateSettings`. Only settings this client honours
//! are listed; the rest stay in the web's settings.
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Switch};
use serde_json::{Value, json};

use crate::theme::{self, Look};
use crate::{App, Env, HANDLE, Popup};
use t3_client::session::Status;

/// `RuntimeMode`, labelled as the web's `runtimeModeConfig`.
const ACCESS_MODES: &[(&str, &str, &str)] = &[
    (
        "approval-required",
        "Supervised",
        "Ask before commands and file changes.",
    ),
    (
        "auto-accept-edits",
        "Auto-accept edits",
        "Auto-approve edits, ask before other actions.",
    ),
    (
        "auto",
        "Auto",
        "Supported providers approve routine actions; others still ask.",
    ),
    (
        "full-access",
        "Full access",
        "Allow commands and edits without prompts.",
    ),
];

impl App {
    pub fn toggle_settings(&mut self) {
        self.settings_open = !self.settings_open;
        self.popup = None;
        self.composer.focused = !self.settings_open;
    }

    /// The machine whose server settings the page shows.
    fn settings_env(&self) -> Option<&Env> {
        let picked = self
            .settings_env
            .as_deref()
            .and_then(|id| self.env_index(id));
        self.envs.get(picked.unwrap_or(0))
    }

    /// Patches the server's settings; the reply (or a `settingsUpdated` event) brings the new
    /// values, so the screen only ever shows what the server holds.
    pub fn update_settings(&mut self, patch: Value) {
        let (Some(env), Some(handle)) = (self.settings_env(), HANDLE.get().cloned()) else {
            return;
        };
        let (Some(session), env_id) = (env.session.clone(), env.cred.environment_id.clone()) else {
            return;
        };
        session.call(
            "server.updateSettings",
            json!({ "patch": patch }),
            move |result| {
                handle.update(move |s: &mut App, tx| match result {
                    Ok(settings) => {
                        let env = s.env_index(&env_id);
                        if let Some(c) = env.and_then(|i| s.envs[i].config.as_mut()) {
                            c.settings = settings;
                        }
                    }
                    Err(e) => s.toast(format!("Couldn't save the setting: {e}"), tx),
                });
            },
        );
    }

    fn server_setting(&self, key: &str) -> Option<&Value> {
        self.settings_env()?.config.as_ref()?.settings.get(key)
    }

    /// Loads the selected machine's archived threads (`orchestration.getArchivedShellSnapshot`).
    pub fn load_archived(&mut self) {
        let (Some(env), Some(handle)) = (self.settings_env(), HANDLE.get().cloned()) else {
            return;
        };
        let (Some(session), env_id) = (env.session.clone(), env.cred.environment_id.clone()) else {
            return;
        };
        self.archived = Some(Archived {
            env_id: env_id.clone(),
            loading: true,
            error: None,
            threads: Vec::new(),
        });
        session.call(
            "orchestration.getArchivedShellSnapshot",
            json!({}),
            move |result| {
                handle.update(move |s: &mut App, _| {
                    let Some(a) = s.archived.as_mut().filter(|a| a.env_id == env_id) else {
                        return;
                    };
                    a.loading = false;
                    match result {
                        Ok(v) => {
                            let project = |id: &str| {
                                v["projects"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .find(|p| p["id"] == id)
                                    .and_then(|p| p["title"].as_str())
                                    .unwrap_or("")
                                    .to_owned()
                            };
                            let mut threads: Vec<ArchivedThread> = v["threads"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|t| t["deletedAt"].is_null())
                                .map(|t| ArchivedThread {
                                    id: t["id"].as_str().unwrap_or("").to_owned(),
                                    title: t["title"].as_str().unwrap_or("").to_owned(),
                                    project: project(t["projectId"].as_str().unwrap_or("")),
                                    archived_at: t["archivedAt"].as_str().unwrap_or("").to_owned(),
                                })
                                .collect();
                            threads.sort_by(|a, b| b.archived_at.cmp(&a.archived_at));
                            a.threads = threads;
                        }
                        Err(e) => a.error = Some(e),
                    }
                });
            },
        );
    }

    /// Unarchives one thread; it returns to the sidebar through the shell stream.
    pub fn restore(&mut self, thread: &str) {
        let Some(a) = self.archived.as_mut() else {
            return;
        };
        a.threads.retain(|t| t.id != thread);
        let env_id = a.env_id.clone();
        if let Some(env) = self.env_index(&env_id) {
            self.thread_action(env, thread, "thread.unarchive", json!({}));
        }
    }

    /// Pairs another machine from a link, keeping the ones already paired.
    pub fn add_machine(&mut self) {
        let text = self.add_link.text.trim().to_owned();
        if self.adding || text.is_empty() {
            return;
        }
        let Some(handle) = HANDLE.get().cloned() else {
            return;
        };
        self.adding = true;
        self.add_error = None;
        std::thread::spawn(move || {
            let result = t3_client::session::pair_blocking(&text).map_err(|e| e.to_string());
            handle.update(move |s: &mut App, tx| {
                s.adding = false;
                match result {
                    Ok(cred) => {
                        let label = cred.label.clone();
                        s.add_link = rune::TextEdit::new("");
                        s.start(cred);
                        s.toast(format!("Paired with {label}"), tx);
                    }
                    Err(e) => s.add_error = Some(e),
                }
            });
        });
    }
}

pub fn page(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let back = IconButton::new(&crate::view::BACK)
        .variant(IconButtonVariant::GhostMuted)
        .label("Close settings")
        .shortcut("Esc")
        .on_press(|s: &mut App| s.toggle_settings())
        .build(k)
        .key("settings-back");
    let header = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .child(back)
        .child(text("Settings").font(k.ty.body_medium).color(look.text));

    let device = section(look, "This device")
        .child(setting_row(
            look,
            "Theme",
            if s.prefs.theme == "omarchy" {
                "Follows Omarchy's theme as it changes."
            } else {
                ""
            },
            theme_select(s, look, layer),
        ))
        .child(setting_row(
            look,
            "Appearance",
            "Light or dark, or follow the system.",
            segmented(
                "mode",
                &["System", "Light", "Dark"],
                ["system", "light", "dark"]
                    .iter()
                    .position(|m| *m == s.prefs.mode)
                    .unwrap_or(0),
                |s, i, tx| {
                    s.prefs.set_mode(["system", "light", "dark"][i]);
                    tx.color_motion(rune::Spring::new(0.4, 0.0));
                    s.rebuild_looks();
                },
                k,
            ),
        ))
        .child(setting_row(
            look,
            "Desktop notifications",
            "When a thread needs you or finishes while T3 Code is in the background.",
            Switch::new(s.prefs.notifications)
                .label("Desktop notifications")
                .hide_label()
                .on_toggle(|s: &mut App| {
                    let on = !s.prefs.notifications;
                    s.prefs.set_notifications(on)
                })
                .build(k)
                .key("notifications"),
        ))
        .child(setting_row(
            look,
            "Follow-ups while an agent works",
            "Ctrl+Enter does the other for one message. Waiting approvals always steer.",
            segmented(
                "follow-up",
                &["Steer", "Queue"],
                usize::from(s.prefs.follow_up == "queue"),
                |s, i, _| {
                    s.prefs
                        .set_follow_up(if i == 1 { "queue" } else { "steer" })
                },
                k,
            ),
        ));

    // The machine picker shows even when the picked machine is offline, so another can be chosen.
    let picker = (s.envs.len() > 1).then(|| {
        section(look, "Server settings").child(setting_row(
            look,
            "Machine",
            "These settings belong to one machine's server.",
            machine_select(s, look, layer),
        ))
    });
    let body = if s.settings_env().and_then(|e| e.config.as_ref()).is_none() {
        col().w_full().child(
            text("That machine isn't connected, so its settings can't be read")
                .font(k.ty.ui)
                .color(look.muted),
        )
    } else {
        let mode = s
            .server_setting("defaultRuntimeMode")
            .and_then(Value::as_str)
            .unwrap_or("full-access");
        let streaming = s
            .server_setting("responseStreamingMode")
            .and_then(Value::as_str)
            .unwrap_or("paragraph");
        let flag = |key: &str, default: bool| {
            s.server_setting(key)
                .and_then(Value::as_bool)
                .unwrap_or(default)
        };
        let threads = section(look, "New threads").child(setting_row(
            look,
            "Access",
            ACCESS_MODES
                .iter()
                .find(|m| m.0 == mode)
                .map_or("", |m| m.2),
            access_select(s, mode, look, layer),
        ));
        let responses = section(look, "Responses").child(setting_row(
            look,
            "Streaming",
            if streaming == "paragraph" {
                "Each paragraph or code block appears as soon as it is complete."
            } else {
                "Text appears once the agent finishes its turn."
            },
            segmented(
                "streaming",
                &["Whole turn", "By paragraph"],
                usize::from(streaming == "paragraph"),
                |s, i, _| {
                    s.update_settings(json!({
                        "responseStreamingMode": if i == 1 { "paragraph" } else { "turn" }
                    }))
                },
                k,
            ),
        ));
        let updates = section(look, "Updates")
            .child(switch_row(
                look,
                "Check for provider updates",
                "See when Codex, Claude and the others have a newer version.",
                "enableProviderUpdateChecks",
                flag("enableProviderUpdateChecks", true),
            ))
            .child(switch_row(
                look,
                "Continue threads after a server update",
                "Threads that were working pick up where they left off.",
                "continueThreadsAfterServerUpdate",
                flag("continueThreadsAfterServerUpdate", true),
            ));
        let body = col().w_full().gap(20.0);
        body.child(threads)
            .child(responses)
            .child(updates)
            .child(archived_section(s, look))
    };

    let mut connection = section(look, "Machines");
    for (i, e) in s.envs.iter().enumerate() {
        let env_id = e.cred.environment_id.clone();
        let state = match &e.status {
            Status::Connected => "Connected".to_owned(),
            Status::Connecting => "Connecting…".to_owned(),
            Status::Reconnecting { reason, .. } => format!("Reconnecting: {reason}"),
        };
        connection = connection.child(
            setting_row(
                look,
                &s.env_name(i),
                &format!("{} · {state}", e.cred.http_base.trim_end_matches('/')),
                Button::new("Unpair")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press_tx(move |s: &mut App, tx| {
                        let Some(i) = s.env_index(&env_id) else {
                            return;
                        };
                        if s.envs.len() == 1 {
                            s.settings_open = false;
                        }
                        s.forget(i, tx);
                    })
                    .build(k)
                    .key(format!("unpair-{i}")),
            )
            .key(format!("machine-{i}")),
        );
    }
    let field = rune_kit::TextInput::new(&s.add_link)
        .label("Pairing link")
        .placeholder("Pairing link from another machine (t3 pair, or Settings → Connections)")
        .on_edit(|s: &mut App| &mut s.add_link)
        .on_submit(|s: &mut App, _| s.add_machine())
        .build(k)
        .key("add-link");
    let add = Button::new(if s.adding { "Pairing…" } else { "Add" })
        .variant(if s.adding {
            ButtonVariant::Loading
        } else {
            ButtonVariant::Outline
        })
        .control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.add_machine())
        .build(k)
        .key("add-machine");
    let mut adder = col().w_full().gap(6.0).child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(field.grow(1.0))
            .child(add),
    );
    if let Some(e) = &s.add_error {
        adder = adder.child(text(e.clone()).font(k.ty.small).color(look.error_fg).wrap());
    }
    connection = connection.child(adder);

    col()
        .key("settings")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .max_w(720.0)
                .px(24.0)
                .py(20.0)
                .gap(20.0)
                .child(header)
                .child(device)
                .children(picker)
                .child(body)
                .child(connection),
        )
}

fn section(look: &Look, title: &str) -> El {
    let k = &look.kit;
    col().w_full().gap(8.0).child(
        text(title.to_owned())
            .font(k.ty.small_medium)
            .color(look.muted),
    )
}

fn setting_row(look: &Look, title: &str, description: &str, control: El) -> El {
    let k = &look.kit;
    let mut words = col()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(2.0)
        .child(text(title.to_owned()).font(k.ty.ui).color(look.text));
    if !description.is_empty() {
        words = words.child(
            text(description.to_owned())
                .font(k.ty.small)
                .color(look.muted)
                .wrap(),
        );
    }
    row()
        .w_full()
        .px(14.0)
        .py(12.0)
        .gap(16.0)
        .center_y()
        .radius(k.radius.lg)
        .fill(look.surface)
        .border(1.0, look.border)
        .child(words)
        .child(control)
}

fn switch_row(look: &Look, title: &str, description: &str, key: &'static str, on: bool) -> El {
    let k = &look.kit;
    setting_row(
        look,
        title,
        description,
        Switch::new(on)
            .label(title.to_owned())
            .hide_label()
            .on_toggle(move |s: &mut App| s.update_settings(json!({ key: !on })))
            .build(k)
            .key(key),
    )
}

fn segmented(
    key: &str,
    labels: &[&str],
    selected: usize,
    pick: fn(&mut App, usize, &mut rune::Tx),
    k: &rune_kit::Kit,
) -> El {
    rune_kit::SegmentedControl::new(labels, Some(selected))
        .fit()
        .on_pick(move |s: &mut App, i, tx: &mut rune::Tx| pick(s, i, tx))
        .build(k)
        .key(key.to_owned())
}

fn theme_select(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let choices = theme::choices(&s.env_themes);
    let labels: Vec<String> = choices.iter().map(|(_, l)| l.clone()).collect();
    let current = choices
        .iter()
        .find(|(id, _)| *id == s.prefs.theme)
        .map(|(_, l)| l.clone());
    rune_kit::Select::new("settings-theme", &labels)
        .current(current.as_deref())
        .open(s.popup == Some(Popup::SettingsTheme))
        .on_toggle(|s: &mut App| s.popup = toggled(s.popup, Popup::SettingsTheme))
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(move |s: &mut App, label: &str, tx: &mut rune::Tx| {
            if let Some((id, _)) = choices.iter().find(|(_, l)| l == label) {
                s.set_theme(id, tx);
            }
        })
        .build(&look.kit, layer)
        .key("settings-theme-select")
}

fn access_select(s: &App, mode: &str, look: &Look, layer: &mut Vec<El>) -> El {
    let labels: Vec<&str> = ACCESS_MODES.iter().map(|m| m.1).collect();
    let current = ACCESS_MODES.iter().find(|m| m.0 == mode).map(|m| m.1);
    rune_kit::Select::new("settings-access", &labels)
        .current(current)
        .open(s.popup == Some(Popup::SettingsAccess))
        .on_toggle(|s: &mut App| s.popup = toggled(s.popup, Popup::SettingsAccess))
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(|s: &mut App, label: &str, _: &mut rune::Tx| {
            s.popup = None;
            if let Some(m) = ACCESS_MODES.iter().find(|m| m.1 == label) {
                s.update_settings(json!({ "defaultRuntimeMode": m.0 }));
            }
        })
        .build(&look.kit, layer)
        .key("settings-access-select")
}

pub struct Archived {
    env_id: String,
    loading: bool,
    error: Option<String>,
    threads: Vec<ArchivedThread>,
}

struct ArchivedThread {
    id: String,
    title: String,
    project: String,
    archived_at: String,
}

fn archived_section(s: &App, look: &Look) -> El {
    let k = &look.kit;
    let mut section = section(look, "Archived threads");
    let showing = s.archived.as_ref().filter(|a| {
        Some(a.env_id.as_str()) == s.settings_env().map(|e| e.cred.environment_id.as_str())
    });
    let Some(a) = showing else {
        return section.child(setting_row(
            look,
            "Archived threads",
            "Archived threads leave the sidebar. Restore one to bring it back.",
            Button::new("Show")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.load_archived())
                .build(k)
                .key("archived-show"),
        ));
    };
    let note = |t: String| text(t).font(k.ty.small).color(look.muted);
    if a.loading {
        return section.child(note("Loading archived threads…".into()));
    }
    if let Some(e) = &a.error {
        return section.child(note(format!("Couldn't load them: {e}")));
    }
    if a.threads.is_empty() {
        return section.child(note("Nothing archived on this machine".into()));
    }
    for (i, t) in a.threads.iter().enumerate() {
        let id = t.id.clone();
        section = section.child(
            setting_row(
                look,
                &t.title,
                &t.project,
                Button::new("Restore")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(move |s: &mut App| s.restore(&id))
                    .build(k)
                    .key(format!("restore-{i}")),
            )
            .key(format!("archived-{}", t.id)),
        );
    }
    section
}

fn machine_select(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    // Names are unique (`env_name` adds the address to a shared one), so a pick maps back.
    let labels: Vec<String> = (0..s.envs.len()).map(|i| s.env_name(i)).collect();
    let current = s
        .settings_env()
        .and_then(|e| s.env_index(&e.cred.environment_id))
        .map(|i| s.env_name(i));
    rune_kit::Select::new("settings-machine", &labels)
        .current(current.as_deref())
        .open(s.popup == Some(Popup::SettingsMachine))
        .on_toggle(|s: &mut App| s.popup = toggled(s.popup, Popup::SettingsMachine))
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(move |s: &mut App, label: &str, _: &mut rune::Tx| {
            s.popup = None;
            if let Some(i) = (0..s.envs.len()).find(|&i| s.env_name(i) == label) {
                s.settings_env = Some(s.envs[i].cred.environment_id.clone());
            }
        })
        .build(&look.kit, layer)
        .key("settings-machine-select")
}

fn toggled(current: Option<Popup>, popup: Popup) -> Option<Popup> {
    (current != Some(popup)).then_some(popup)
}
