//! Thread actions from the sidebar's context menu (pin, settle, snooze, mark unread, rename,
//! archive), the snooze presets, and visit tracking, which keeps the read state every client
//! shares on the server.
use chrono::{Datelike, Duration, Local, NaiveTime, TimeZone};
use rune::{El, PointerPress, TextEdit, col, el, text};
use rune_kit::{Button, ButtonVariant, ControlSize, MenuItem, TextInput, context_menu};
use serde_json::{Value, json};
use t3_client::protocol::{ThreadShell, thread_command};
use t3_client::shelves::{has_unseen_completion, iso_ms, latest_run_completed_at};

use crate::theme::Look;
use crate::{App, focused, now_secs};

/// The web throttles visits while a run streams: each one is a command plus a shell broadcast.
const VISIT_THROTTLE_MS: i64 = 10_000;

/// The thread a context menu was opened on: (environment, thread id, where).
pub struct ThreadMenu {
    pub env: usize,
    pub thread: String,
    pub at: PointerPress,
}

pub struct Rename {
    pub env: usize,
    pub thread: String,
    pub title: TextEdit,
}

/// The open thread's visit watermark (`createThreadVisitTracker`).
pub struct Visit {
    env_id: String,
    thread: String,
    open_pending: bool,
    last_dispatched: Option<i64>,
    last_dispatch_at: i64,
    /// A throttled visit waits until this wall-clock millisecond.
    pub due: Option<i64>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// One snooze choice, resolved against the clock (`resolveSnoozePresets`).
pub struct SnoozeChoice {
    pub label: String,
    pub when: String,
    pub until: String,
}

/// The server's `snoozePresets` (the web's defaults when unset), soonest first, one per minute.
pub fn snooze_choices(presets: Option<&Value>) -> Vec<SnoozeChoice> {
    let defaults = json!([
        { "kind": "after", "minutes": 60 },
        { "kind": "after", "minutes": 180 },
        { "kind": "at", "day": "today", "time": "18:00" },
        { "kind": "at", "day": "tomorrow", "time": "09:00" },
        { "kind": "at", "day": "monday", "time": "09:00" },
    ]);
    let list = presets
        .and_then(Value::as_array)
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| defaults.as_array().unwrap());
    let now = Local::now();
    let mut out: Vec<(i64, SnoozeChoice)> = Vec::new();
    for p in list {
        let (wake, label, names_day) = match p["kind"].as_str() {
            Some("after") => {
                let Some(m) = p["minutes"].as_i64().filter(|m| *m > 0) else {
                    continue;
                };
                (
                    now + Duration::minutes(m),
                    format!("In {}", duration_label(m)),
                    false,
                )
            }
            Some("at") => {
                let (Some(day), Some(time)) = (p["day"].as_str(), p["time"].as_str()) else {
                    continue;
                };
                let Ok(t) = NaiveTime::parse_from_str(time, "%H:%M") else {
                    continue;
                };
                let ahead = days_ahead(day, now.weekday().num_days_from_sunday());
                let date = now.date_naive() + Duration::days(ahead);
                let Some(wake) = Local.from_local_datetime(&date.and_time(t)).earliest() else {
                    continue;
                };
                // A "today" choice whose time has passed waits for tomorrow.
                if wake <= now {
                    continue;
                }
                let label = at_label(day, ahead, t);
                let names_day = label != "Next week";
                (wake, label, names_day)
            }
            _ => continue,
        };
        let time = wake.format("%-I:%M %p").to_string();
        let when = if names_day || wake.date_naive() == now.date_naive() {
            time
        } else {
            format!("{} {time}", wake.format("%a"))
        };
        out.push((
            wake.timestamp() / 60,
            SnoozeChoice {
                label,
                when,
                until: wake
                    .with_timezone(&chrono::Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            },
        ));
    }
    // Stable: equal minutes keep the settings' order, and only the first stays.
    out.sort_by_key(|(minute, _)| *minute);
    out.dedup_by_key(|(minute, _)| *minute);
    out.into_iter().map(|(_, c)| c).collect()
}

fn duration_label(minutes: i64) -> String {
    let plural = |n: i64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    if minutes % (24 * 60) == 0 {
        plural(minutes / (24 * 60), "day")
    } else if minutes % 60 == 0 {
        plural(minutes / 60, "hour")
    } else {
        plural(minutes, "minute")
    }
}

fn days_ahead(day: &str, today: u32) -> i64 {
    const DAYS: [&str; 7] = [
        "sunday",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
    ];
    match day {
        "today" => 0,
        "tomorrow" => 1,
        d => match DAYS.iter().position(|x| *x == d) {
            Some(i) => match (i as i64 - today as i64 + 7) % 7 {
                0 => 7,
                n => n,
            },
            None => 1,
        },
    }
}

fn at_label(day: &str, ahead: i64, t: NaiveTime) -> String {
    use chrono::Timelike;
    if ahead == 0 {
        return match t.hour() {
            h if h < 12 => "This morning",
            h if h < 17 => "This afternoon",
            _ => "This evening",
        }
        .into();
    }
    if ahead == 1 {
        return "Tomorrow".into();
    }
    if day == "monday" {
        return "Next week".into();
    }
    let mut name = day.to_owned();
    name[..1].make_ascii_uppercase();
    if ahead == 7 {
        format!("Next {name}")
    } else {
        name
    }
}

impl App {
    fn thread_shell(&self, env: usize, thread: &str) -> Option<&ThreadShell> {
        self.envs.get(env)?.shell.thread(thread)
    }

    /// Sends a thread command to the environment that owns the thread.
    pub fn thread_action(&mut self, env: usize, thread: &str, kind: &str, extra: Value) {
        self.thread_menu = None;
        if let Some(session) = self.envs.get(env).and_then(|e| e.session.clone()) {
            session.dispatch(thread_command(kind, thread, extra));
        }
    }

    pub fn start_rename(&mut self, env: usize, thread: &str) {
        let title = self
            .thread_shell(env, thread)
            .map(|t| t.title.clone())
            .unwrap_or_default();
        self.thread_menu = None;
        self.composer.focused = false;
        let mut edit = focused(&title);
        edit.set_selection(0, title.len());
        self.rename = Some(Rename {
            env,
            thread: thread.to_owned(),
            title: edit,
        });
    }

    pub fn finish_rename(&mut self, save: bool) {
        let Some(r) = self.rename.take() else { return };
        self.composer.focused = true;
        let title = r.title.text.trim().to_owned();
        if save && !title.is_empty() {
            self.thread_action(
                r.env,
                &r.thread,
                "thread.metadata.update",
                json!({ "title": title }),
            );
        }
    }

    /// Starts tracking the open thread's visits; the first one tells the server it was opened.
    pub fn begin_visit(&mut self) {
        self.visit = self.selected.clone().and_then(|thread| {
            Some(Visit {
                env_id: self.env()?.cred.environment_id.clone(),
                thread,
                open_pending: true,
                last_dispatched: None,
                last_dispatch_at: 0,
                due: None,
            })
        });
        self.track_visit();
    }

    /// `createThreadVisitTracker`'s shell handler: advance the server's read watermark to the
    /// open thread's latest update, at once for an open or a finished run, else throttled.
    pub fn track_visit(&mut self) {
        let Some(v) = self.visit.as_ref() else { return };
        let Some(env) = self.env_index(&v.env_id) else {
            return;
        };
        let Some(t) = self.envs[env].shell.thread(&v.thread) else {
            return;
        };
        // An older server without visit tracking.
        let Some(last_visited) = &t.last_visited_at else {
            return;
        };
        let Some(updated) = iso_ms(&t.updated_at) else {
            return;
        };
        let visited_at = t.updated_at.clone();
        let last_visited = last_visited.as_deref().and_then(iso_ms);
        let unseen = latest_run_completed_at(t)
            .and_then(iso_ms)
            .is_some_and(|done| last_visited.is_none_or(|lv| done > lv));
        let (thread, session) = (v.thread.clone(), self.envs[env].session.clone());
        let v = self.visit.as_mut().unwrap();
        v.due = None;
        let opened = v.open_pending;
        if !opened {
            if last_visited.is_some_and(|lv| lv >= updated) {
                // Already seen, so a later mark-unread stays put.
                if v.last_dispatched.is_none_or(|d| d < updated) {
                    v.last_dispatched = Some(updated);
                }
                return;
            }
            if v.last_dispatched.is_some_and(|d| d >= updated) {
                return;
            }
            let wait = VISIT_THROTTLE_MS - (now_ms() - v.last_dispatch_at);
            if !unseen && wait > 0 {
                v.due = Some(now_ms() + wait);
                return;
            }
        }
        v.open_pending = false;
        v.last_dispatched = Some(updated);
        v.last_dispatch_at = now_ms();
        if let Some(session) = session {
            session.dispatch(thread_command(
                "thread.visit",
                &thread,
                json!({ "visitedAt": visited_at, "opened": opened }),
            ));
        }
    }

    /// From `poll`: sends a throttled visit once it's due; returns when to wake for the next.
    pub fn visit_due(&mut self) -> Option<std::time::Duration> {
        let due = self.visit.as_ref()?.due?;
        let left = due - now_ms();
        if left > 0 {
            return Some(std::time::Duration::from_millis(left as u64));
        }
        self.track_visit();
        None
    }
}

/// The context menu for the thread it was opened on.
pub fn thread_menu(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let menu = context_menu(
        "thread-menu",
        el().key("thread-menu-anchor"),
        s.thread_menu.as_ref().map(|m| m.at),
    )
    .min_width(220.0)
    .on_close(|s: &mut App| s.thread_menu = None);
    let Some(m) = &s.thread_menu else {
        return menu.build(k, layer);
    };
    let Some(t) = s.thread_shell(m.env, &m.thread) else {
        return menu.build(k, layer);
    };
    let (env, id) = (m.env, m.thread.clone());
    let act = |kind: &'static str, extra: Value| {
        let id = id.clone();
        move |s: &mut App| s.thread_action(env, &id, kind, extra.clone())
    };
    let now = now_secs() * 1000;
    let snoozed = t3_client::shelves::is_snoozed(t, now);
    let rename_id = id.clone();
    let mut menu = menu.item(
        MenuItem::new("Rename…").on_select(move |s: &mut App| s.start_rename(env, &rename_id)),
    );
    menu = menu.item(if t.pinned_at.is_some() {
        MenuItem::new("Unpin").on_select(act("thread.unpin", json!({})))
    } else {
        MenuItem::new("Pin").on_select(act("thread.pin", json!({})))
    });
    menu = menu.item(if t.is_settled() {
        MenuItem::new("Unsettle").on_select(act("thread.unsettle", json!({ "reason": "user" })))
    } else {
        MenuItem::new("Settle").on_select(act("thread.settle", json!({})))
    });
    if !has_unseen_completion(t) {
        menu =
            menu.item(MenuItem::new("Mark unread").on_select(act("thread.mark-unread", json!({}))));
    }
    menu = menu.separator();
    if snoozed {
        menu = menu.item(
            MenuItem::new("Wake now")
                .on_select(act("thread.unsnooze", json!({ "reason": "user" }))),
        );
    } else {
        menu = menu.group_label("Snooze");
        let presets = s
            .envs
            .get(env)
            .and_then(|e| e.config.as_ref())
            .and_then(|c| c.settings.get("snoozePresets"));
        for c in snooze_choices(presets) {
            menu = menu.item(
                MenuItem::new(c.label)
                    .shortcut(c.when)
                    .on_select(act("thread.snooze", json!({ "snoozedUntil": c.until }))),
            );
        }
    }
    menu = menu
        .separator()
        .item(MenuItem::new("Archive").on_select(act("thread.archive", json!({}))));
    menu.build(k, layer)
}

/// The rename field, over the window like the palette.
pub fn rename_dialog(s: &App, look: &Look) -> Option<El> {
    let r = s.rename.as_ref()?;
    let k = &look.kit;
    let field = TextInput::new(&r.title)
        .label("Thread title")
        .on_edit(|s: &mut App| match s.rename.as_mut() {
            Some(r) => &mut r.title,
            None => &mut s.composer,
        })
        .on_submit(|s: &mut App, _| s.finish_rename(true))
        .build(k)
        .key("rename-field");
    let save = Button::new("Rename")
        .variant(ButtonVariant::Contrast)
        .control_size(ControlSize::Small)
        .on_press(|s: &mut App| s.finish_rename(true))
        .build(k)
        .key("rename-save");
    let card = col()
        .w(420.0)
        .padding(16.0)
        .gap(12.0)
        .radius(k.radius.lg)
        .fill(look.raised)
        .border(1.0, look.border)
        .child(text("Rename thread").font(k.ty.ui_medium).color(look.text))
        .child(field)
        .child(rune::row().w_full().justify_end().child(save));
    Some(
        col()
            .key("rename-scrim")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .fill(look.canvas.alpha(0.55))
            .on_press(|s: &mut App| s.finish_rename(false))
            .child(card.on_press(|_: &mut App| {})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snooze_labels_follow_the_web() {
        assert_eq!(duration_label(60), "1 hour");
        assert_eq!(duration_label(90), "90 minutes");
        assert_eq!(duration_label(2880), "2 days");
        // Sunday is 0: Monday is tomorrow, a Monday on Monday is next week.
        assert_eq!(days_ahead("monday", 0), 1);
        assert_eq!(days_ahead("monday", 1), 7);
        let nine = NaiveTime::from_hms_opt(9, 0, 0).unwrap();
        assert_eq!(at_label("monday", 1, nine), "Tomorrow");
        assert_eq!(at_label("monday", 7, nine), "Next week");
        assert_eq!(at_label("friday", 7, nine), "Next Friday");
        assert_eq!(at_label("friday", 3, nine), "Friday");
        assert_eq!(
            at_label("today", 0, NaiveTime::from_hms_opt(18, 0, 0).unwrap()),
            "This evening"
        );
    }
}

impl App {
    /// The Settled shelf's "Show more": the next page of older settled threads from every
    /// machine that has more (`orchestration.getOlderSettledThreads`).
    pub fn load_older_settled(&mut self) {
        let Some(handle) = crate::HANDLE.get().cloned() else {
            return;
        };
        for env in &mut self.envs {
            let (Some(cursor), Some(session)) = (env.shell.older_cursor(), env.session.clone())
            else {
                continue;
            };
            let Some(older) = env.shell.older.as_mut().filter(|o| !o.loading) else {
                continue;
            };
            older.loading = true;
            let (env_id, before) = (env.cred.environment_id.clone(), older.before.clone());
            let handle = handle.clone();
            let input = json!({ "before": cursor, "limit": t3_client::store::OLDER_SETTLED_PAGE });
            session.call(
                "orchestration.getOlderSettledThreads",
                input,
                move |result| {
                    handle.update(move |s: &mut App, tx| {
                        let Some(i) = s.env_index(&env_id) else {
                            return;
                        };
                        match result {
                            Ok(v) => {
                                let threads = v["threads"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(|t| {
                                        serde_json::from_value::<ThreadShell>(t.clone()).ok()
                                    })
                                    .collect();
                                s.envs[i].shell.add_older_page(&before, threads);
                            }
                            Err(e) => {
                                if let Some(o) = s.envs[i].shell.older.as_mut() {
                                    o.loading = false;
                                }
                                s.toast(format!("Couldn't load older threads: {e}"), tx);
                            }
                        }
                    });
                },
            );
        }
    }
}
