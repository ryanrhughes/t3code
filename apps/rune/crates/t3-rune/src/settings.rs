//! Settings: this device's preferences, kept in prefs.json, and the server's, read from the
//! config stream and changed with `server.updateSettings`. Only settings this client honours
//! are listed; the rest stay in the web's settings.
use rune::{El, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Switch};
use serde_json::{Value, json};

use crate::theme::{self, Look};
use crate::{App, HANDLE, Popup};

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

    /// Patches the server's settings; the reply (or a `settingsUpdated` event) brings the new
    /// values, so the screen only ever shows what the server holds.
    pub fn update_settings(&mut self, patch: Value) {
        let (Some(session), Some(handle)) = (&self.session, HANDLE.get().cloned()) else {
            return;
        };
        session.call(
            "server.updateSettings",
            json!({ "patch": patch }),
            move |result| {
                handle.update(move |s: &mut App, tx| match result {
                    Ok(settings) => {
                        if let Some(c) = s.config.as_mut() {
                            c.settings = settings;
                        }
                    }
                    Err(e) => s.toast(format!("Couldn't save the setting: {e}"), tx),
                });
            },
        );
    }

    fn server_setting(&self, key: &str) -> Option<&Value> {
        self.config.as_ref()?.settings.get(key)
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

    let body = if s.config.is_none() {
        col().w_full().child(
            text("Connect to a server to change its settings")
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
        col()
            .w_full()
            .gap(20.0)
            .child(threads)
            .child(responses)
            .child(updates)
    };

    let server = s
        .config
        .as_ref()
        .map(|c| c.environment.label.clone())
        .unwrap_or_default();
    let connection = section(look, "Connection").child(setting_row(
        look,
        if server.is_empty() {
            "Not connected"
        } else {
            &server
        },
        s.cred.as_ref().map_or("", |c| c.http_base.as_str()),
        Button::new("Unpair")
            .variant(ButtonVariant::Outline)
            .control_size(ControlSize::Small)
            .on_press(|s: &mut App| {
                s.settings_open = false;
                s.forget();
            })
            .build(k)
            .key("settings-unpair"),
    ));

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

fn toggled(current: Option<Popup>, popup: Popup) -> Option<Popup> {
    (current != Some(popup)).then_some(popup)
}
