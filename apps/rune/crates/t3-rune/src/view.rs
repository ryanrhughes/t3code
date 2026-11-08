//! Every screen. Views are pure functions of `App`; handlers mutate it between frames.
use std::borrow::Cow;
use std::rc::Rc;

use rune::{Cx, El, GrowthSpec, Icon, ScrollSpec, Spring, col, el, row, text};
use rune_kit::markdown::Markdown;
use rune_kit::shell::{
    PaneMotion, RowStatus, Sidebar, SidebarHeader, SidebarRow, SidebarToggle, SplitView, Toolbar,
};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Menu, MenuItem,
    Spinner, TextInput, Toast, ToastRegion, glyphs,
};
use t3_client::protocol::{ModelSelection, TurnItem};
use t3_client::session::Status;

use crate::theme::{self, Look};
use crate::{App, Inspector, Popup, Screen};

static PLUS: Icon = Icon {
    name: "t3-plus",
    view: 16.0,
    body: r#"<path d="M8 3.5V12.5M3.5 8H12.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#,
};
/// The web's working marker (`CircleDashedIcon`): static, so a working thread never repaints.
static WORKING: Icon = Icon {
    name: "t3-working",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.4" stroke-dasharray="2.4 2.2" fill="none"/>"#,
};
static STOP: Icon = Icon {
    name: "t3-stop",
    view: 16.0,
    body: r#"<rect x="4.5" y="4.5" width="7" height="7" rx="1.5" fill="currentColor"/>"#,
};
static SEND: Icon = Icon {
    name: "t3-send",
    view: 16.0,
    body: r#"<path d="M8 12.5V3.5M8 3.5L4 7.5M8 3.5L12 7.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};
static PALETTE: Icon = Icon {
    name: "t3-palette",
    view: 16.0,
    body: r#"<circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="6" cy="6.5" r="1" fill="currentColor"/><circle cx="9.5" cy="5.75" r="1" fill="currentColor"/><circle cx="10.5" cy="9" r="1" fill="currentColor"/>"#,
};
static SETTINGS: Icon = Icon {
    name: "t3-settings",
    view: 16.0,
    body: r#"<path d="M3 4.5H9M12 4.5H13M3 11.5H4M7 11.5H13" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/><circle cx="10.5" cy="4.5" r="1.5" stroke="currentColor" stroke-width="1.3" fill="none"/><circle cx="5.5" cy="11.5" r="1.5" stroke="currentColor" stroke-width="1.3" fill="none"/>"#,
};

pub(crate) static BACK: Icon = Icon {
    name: "t3-back",
    view: 16.0,
    body: r#"<path d="M10 3.5L5.5 8L10 12.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

static DIFF: Icon = Icon {
    name: "t3-diff",
    view: 16.0,
    body: r#"<path d="M5 3.5V9.5M2 6.5H8M3 12.5H13M10 3.5H14" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" fill="none"/>"#,
};
static TERMINAL: Icon = Icon {
    name: "t3-terminal",
    view: 16.0,
    body: r#"<path d="M3.5 5L6.5 8L3.5 11M8 11.5H12.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};
static PENCIL: Icon = Icon {
    name: "t3-pencil",
    view: 16.0,
    body: r#"<path d="M10.5 3.5L12.5 5.5L6 12H4V10L10.5 3.5Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#,
};

/// Content width of the transcript and composer, as the web caps it.
const COLUMN: f32 = 768.0;

pub fn root(s: &App, cx: &mut Cx<App>) -> El {
    let look = s.looks.get(cx.system_dark());
    let mut layer = Vec::new();
    let body = match &s.screen {
        Screen::Pair {
            link,
            error,
            busy,
            local,
        } => pair_screen(look, link, error.as_deref(), *busy, local.as_ref()),
        Screen::Main => main_screen(s, cx, look, &mut layer),
    };
    if s.palette.is_some() {
        layer.push(palette(s, look));
    }
    if let Some(dialog) = crate::actions::rename_dialog(s, look) {
        layer.push(dialog);
    }
    if let Some(picker) = crate::picker::overlay(s, look) {
        layer.push(picker);
    }
    if !s.toasts.is_empty() {
        let k = &look.kit;
        let region = s.toasts.iter().fold(ToastRegion::new(), |r, (id, msg, _)| {
            let id = *id;
            r.toast(
                Toast::new(format!("toast-{id}"), msg.clone())
                    .closable()
                    .on_close(move |s: &mut App| s.toasts.retain(|t| t.0 != id))
                    .build(k),
            )
        });
        layer.push(region.build(k));
    }
    rune_kit::root(body, layer)
        // Notifications go out only while the window is in the background.
        .on_window_focus(|s: &mut App, focused, _| s.window_focused = focused)
}

// ---------------------------------------------------------------------------------------------
// Pairing

fn pair_screen(
    look: &Look,
    link: &rune::TextEdit,
    error: Option<&str>,
    busy: bool,
    local: Option<&url::Url>,
) -> El {
    let k = &look.kit;
    let field = TextInput::new(link)
        .label("Pairing link")
        .placeholder("http://host:3773/pair#token=…")
        .on_edit(|s: &mut App| match &mut s.screen {
            Screen::Pair { link, .. } => link,
            Screen::Main => &mut s.composer,
        })
        .on_submit(|s: &mut App, _| s.pair())
        .build(k)
        .key("pair-link");
    let button = Button::new(if busy { "Pairing…" } else { "Pair" })
        .variant(if busy {
            ButtonVariant::Loading
        } else {
            ButtonVariant::Contrast
        })
        .control_size(ControlSize::Large)
        .on_press(|s: &mut App| s.pair())
        .build(k)
        .key("pair-button");
    let mut card = col()
        .key("pair-card")
        .w_full()
        .max_w(460.0)
        .padding(28.0)
        .gap(14.0)
        .radius(k.radius.lg)
        .fill(look.surface)
        .border(1.0, look.border)
        .child(text("Connect to T3 Code").font(k.ty.body_medium).color(look.text))
        .child(
            text("Paste a pairing link from `t3 pair`, the desktop app's Connections settings, or a server's startup output. Local, LAN, Tailscale and T3 Connect links all work.")
                .font(k.ty.ui)
                .color(look.muted)
                .wrap(),
        )
        .child(field)
        .child(row().justify_end().child(button));
    // On a machine that runs T3 Code, connecting needs no link: the local `t3` CLI mints one.
    if let Some(origin) = local {
        let host = origin
            .host_str()
            .zip(origin.port())
            .map_or_else(|| origin.to_string(), |(h, p)| format!("{h}:{p}"));
        let connect = Button::new(if busy {
            "Connecting…".to_owned()
        } else {
            format!("Connect to this machine ({host})")
        })
        .variant(if busy {
            ButtonVariant::Loading
        } else {
            ButtonVariant::Contrast
        })
        .control_size(ControlSize::Large)
        .on_press(|s: &mut App| s.connect_local())
        .build(k)
        .key("pair-local");
        card = card.child(
            col()
                .w_full()
                .pt(6.0)
                .gap(8.0)
                .child(
                    text("Or use the server running here")
                        .font(k.ty.small)
                        .color(look.muted),
                )
                .child(connect),
        );
    }
    if let Some(e) = error {
        card = card.child(
            text(e.to_owned())
                .font(k.ty.small)
                .color(look.error_fg)
                .wrap()
                .key("pair-error"),
        );
    }
    col()
        .key("pair")
        .w_full()
        .h_full()
        .fill(look.canvas)
        .center()
        .padding(24.0)
        .child(card)
}

// ---------------------------------------------------------------------------------------------
// Main: sidebar + thread

fn main_screen(s: &App, cx: &mut Cx<App>, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let title = match (&s.selected, &s.thread) {
        _ if s.settings_open => "Settings".to_owned(),
        (Some(_), Some(t)) => t.thread.title.clone(),
        (Some(id), None) => s
            .shell()
            .thread(id)
            .map(|t| t.title.clone())
            .unwrap_or_default(),
        (None, _) => "New thread".to_owned(),
    };
    let showing = |tab| s.diff_pane.is_open() && s.inspector == tab;
    let diff_toggle = IconButton::new(&DIFF)
        .variant(IconButtonVariant::GhostMuted)
        .label(if showing(Inspector::Changes) {
            "Hide changes"
        } else {
            "Show changes"
        })
        .shortcut("Ctrl D")
        .expanded(showing(Inspector::Changes))
        .on_press(|s: &mut App| s.toggle_diff())
        .build(k)
        .key("diff-toggle");
    let files_toggle = IconButton::new(&glyphs::FOLDER)
        .variant(IconButtonVariant::GhostMuted)
        .label(if showing(Inspector::Files) {
            "Hide files"
        } else {
            "Show files"
        })
        .expanded(showing(Inspector::Files))
        .on_press(|s: &mut App| s.toggle_inspector(Inspector::Files))
        .build(k)
        .key("files-toggle");
    let toolbar = Toolbar::new().title(title).trailing(
        row()
            .gap(4.0)
            .center_y()
            .child(files_toggle)
            .child(diff_toggle)
            .child(theme_menu(s, look, layer)),
    );
    let main = col()
        .key("main")
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(look.canvas);
    let main = if s.settings_open {
        main.child(crate::settings::page(s, look, layer))
    } else if let Some(bar) = crate::find::bar(s, look) {
        main.child(row().w_full().justify_end().px(16.0).pt(8.0).child(bar))
            .child(timeline(s, look))
            .child(composer(s, look, layer))
    } else {
        main.child(timeline(s, look))
            .child(composer(s, look, layer))
    };
    let working = s
        .envs
        .iter()
        .any(|e| e.shell.threads.iter().any(|t| t.is_working()));
    SplitView::new(main)
        .sidebar(
            &s.sidebar,
            |s: &mut App| &mut s.sidebar,
            sidebar(s, look, layer),
        )
        .inspector(
            &s.diff_pane,
            |s: &mut App| &mut s.diff_pane,
            inspector(s, look, layer),
        )
        .toolbar(toolbar)
        .titlebar_inset(cx.titlebar_inset())
        .frame_inset(8.0)
        .toolbar_in_panel()
        .sidebar_padding(12.0, 8.0)
        .sidebar_toggle(
            SidebarToggle::new()
                .size(ControlSize::Large)
                .past_lights(16.0)
                .edge(18.0)
                .gap(4.0)
                .in_header()
                .unread(working),
        )
        .pane_motion(
            PaneMotion::new()
                .spring(Spring::new(0.3, 0.0))
                .scale_with_width(480.0, 0.15, 0.3)
                .fade_tail(0.15),
        )
        .reduced_motion(cx.is_reduced_motion())
        .build(k)
        .on_layout(|s: &mut App, r: [f32; 4]| s.fit_width(r[2]))
}

#[derive(Clone)]
enum Entry {
    Project {
        env: usize,
        id: String,
        title: String,
        count: usize,
        collapsed: bool,
    },
    Thread {
        env: usize,
        id: String,
        title: String,
        status: Option<Status3>,
        age: String,
    },
    /// "Show more" under the Settled shelf, while servers hold older settled threads.
    More { loading: bool, remaining: u64 },
    /// A shelf's header: Pinned above the projects, Snoozed and Settled under them.
    Shelf {
        shelf: Shelf,
        count: usize,
        /// `None` for Pinned, which doesn't fold.
        expanded: Option<bool>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    Pinned,
    Snoozed,
    Settled,
}

/// The web's sections (`resolveSidebarThreadSection`): snoozed, else settled, else pinned,
/// else active in its project's folder.
fn section(t: &t3_client::protocol::ThreadShell, now_ms: i64) -> Option<Shelf> {
    if t3_client::shelves::is_snoozed(t, now_ms) {
        Some(Shelf::Snoozed)
    } else if t.is_settled() {
        Some(Shelf::Settled)
    } else if t.pinned_at.is_some() {
        Some(Shelf::Pinned)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
enum Status3 {
    Busy,
    Attention,
    /// Finished since the user last looked (`hasUnseenCompletion`).
    Unread,
}

/// The key `collapsed` keeps a project's folder under: projects are per environment.
pub fn folder_key(s: &App, env: usize, project: &str) -> String {
    let id = s
        .envs
        .get(env)
        .map_or("", |e| e.cred.environment_id.as_str());
    format!("{id}:{project}")
}

/// The sidebar as one flat list (shelves, project headers and their threads) so it virtualises.
/// Every paired environment's threads are listed; with more than one, folders name their machine.
/// Every thread row in sidebar order, for stepping to the previous or next thread.
pub fn thread_order(s: &App) -> Vec<(usize, String)> {
    entries(s)
        .into_iter()
        .filter_map(|e| match e {
            Entry::Thread { env, id, .. } => Some((env, id)),
            _ => None,
        })
        .collect()
}

fn entries(s: &App) -> Vec<Entry> {
    let now = now_secs();
    let now_ms = now * 1000;
    let many = s.envs.len() > 1;
    type Row<'a> = (usize, &'a t3_client::protocol::ThreadShell);
    let (mut pinned, mut snoozed, mut settled): (Vec<Row>, Vec<Row>, Vec<Row>) = Default::default();
    let mut folders = Vec::new();
    for (env, e) in s.envs.iter().enumerate() {
        for t in e.shell.threads.iter().filter(|t| t.is_sidebar_row()) {
            match section(t, now_ms) {
                Some(Shelf::Pinned) => pinned.push((env, t)),
                Some(Shelf::Snoozed) => snoozed.push((env, t)),
                Some(Shelf::Settled) => settled.push((env, t)),
                None => {}
            }
        }
        for p in e.shell.sidebar_projects() {
            let threads: Vec<_> = e
                .shell
                .project_threads(&p.id)
                .into_iter()
                .filter(|t| section(t, now_ms).is_none())
                .collect();
            // As the web's folders: one whose threads all moved to shelves goes, unless pinned.
            if threads.is_empty() && p.pinned_at.is_none() {
                continue;
            }
            let latest = threads
                .first()
                .map_or(p.updated_at.as_str(), |t| t.updated_at.as_str());
            folders.push((p.pinned_at.is_some(), latest.to_owned(), env, p, threads));
        }
    }
    let mut out = Vec::new();
    // A shelf's rows; a folded shelf still shows the open thread, as the web's do.
    let shelf_rows = |out: &mut Vec<Entry>, rows: Vec<Row>, expanded: bool| {
        out.extend(
            rows.into_iter()
                .filter(|(env, t)| {
                    expanded || (*env == s.focus && s.selected.as_deref() == Some(t.id.as_str()))
                })
                .map(|(env, t)| thread_entry(s, env, t, now)),
        );
    };
    if !pinned.is_empty() {
        let mut keyed: Vec<(&str, &t3_client::protocol::ThreadShell)> = pinned
            .iter()
            .map(|(env, t)| (s.envs[*env].cred.environment_id.as_str(), *t))
            .collect();
        t3_client::shelves::sort_pinned(&mut keyed);
        out.push(Entry::Shelf {
            shelf: Shelf::Pinned,
            count: keyed.len(),
            expanded: None,
        });
        let env_of = |id: &str| s.env_index(id).unwrap_or(0);
        shelf_rows(
            &mut out,
            keyed.into_iter().map(|(e, t)| (env_of(e), t)).collect(),
            true,
        );
    }
    // Pinned folders first, then the most recently active, across machines.
    folders.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for (_, _, env, p, threads) in folders {
        let collapsed = s.collapsed.contains(&folder_key(s, env, &p.id));
        out.push(Entry::Project {
            env,
            id: p.id.clone(),
            title: if many {
                format!("{} · {}", p.title, s.env_name(env))
            } else {
                p.title.clone()
            },
            count: threads.len(),
            collapsed,
        });
        if collapsed {
            continue;
        }
        out.extend(threads.into_iter().map(|t| thread_entry(s, env, t, now)));
    }
    if !snoozed.is_empty() {
        // Soonest to wake first, as the web's.
        let wake = |t: &t3_client::protocol::ThreadShell| {
            t.snoozed_until
                .as_deref()
                .and_then(t3_client::shelves::iso_ms)
                .unwrap_or(0)
        };
        snoozed.sort_by_key(|(_, t)| wake(t));
        let expanded = s.prefs.snoozed_expanded;
        out.push(Entry::Shelf {
            shelf: Shelf::Snoozed,
            count: snoozed.len(),
            expanded: Some(expanded),
        });
        shelf_rows(&mut out, snoozed, expanded);
    }
    // The count: settled rows above each machine's cut, plus what each server left out.
    let count: u64 = settled
        .iter()
        .filter(|(env, t)| !s.envs[*env].shell.is_below_cut(t))
        .count() as u64
        + s.envs
            .iter()
            .filter_map(|e| e.shell.older.as_ref())
            .map(|o| o.total)
            .sum::<u64>();
    // Older rows show only above the deepest point a machine with more to load has reached,
    // so the merged list stays in order while pages arrive (`olderSettledThreads.ts`).
    let frontier = s.envs.iter().filter_map(|e| e.shell.older_frontier()).max();
    for (env, e) in s.envs.iter().enumerate() {
        for t in e.shell.older_rows() {
            let at = t.settled_at.as_deref().and_then(t3_client::shelves::iso_ms);
            if frontier.is_none_or(|f| at.is_some_and(|at| at >= f)) {
                settled.push((env, t));
            }
        }
    }
    if count > 0 || !settled.is_empty() {
        let at = |t: &t3_client::protocol::ThreadShell| {
            t.settled_at.clone().unwrap_or_else(|| t.updated_at.clone())
        };
        settled.sort_by(|a, b| at(b.1).cmp(&at(a.1)).then_with(|| a.1.id.cmp(&b.1.id)));
        let expanded = s.prefs.settled_expanded;
        out.push(Entry::Shelf {
            shelf: Shelf::Settled,
            count: count.max(settled.len() as u64) as usize,
            expanded: Some(expanded),
        });
        let shown = settled.len() as u64;
        shelf_rows(&mut out, settled, expanded);
        let loading = s
            .envs
            .iter()
            .any(|e| e.shell.older.as_ref().is_some_and(|o| o.loading));
        let more = s.envs.iter().any(|e| e.shell.older_cursor().is_some());
        if expanded && more {
            out.push(Entry::More {
                loading,
                remaining: count.saturating_sub(shown),
            });
        }
    }
    out
}

fn thread_entry(s: &App, env: usize, t: &t3_client::protocol::ThreadShell, now: i64) -> Entry {
    let open = env == s.focus && s.selected.as_deref() == Some(t.id.as_str());
    let status = if t.needs_attention() {
        Some(Status3::Attention)
    } else if t.is_working() {
        Some(Status3::Busy)
    } else if !open && t3_client::shelves::has_unseen_completion(t) {
        Some(Status3::Unread)
    } else {
        None
    };
    let detail = match (
        status,
        t.activity_run_started_at.as_deref().and_then(parse_iso),
    ) {
        (Some(Status3::Busy), Some(start)) => elapsed(s.clock - start),
        _ => age(&t.updated_at, now),
    };
    Entry::Thread {
        env,
        id: t.id.clone(),
        title: t.title.clone(),
        status,
        age: detail,
    }
}

fn sidebar(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let rows = Rc::new(entries(s));
    let new_row = SidebarRow::new("New thread")
        .icon(&PLUS)
        .detail("Ctrl N")
        .height(32.0)
        .selected(s.selected.is_none())
        .on_select(|s: &mut App, _| s.new_thread(None))
        .build(k)
        .key("new-thread");
    // One machine: its name and state. Several: how many are up, and which ones aren't.
    let down: Vec<String> = s
        .envs
        .iter()
        .enumerate()
        .filter(|(_, e)| !matches!(e.status, Status::Connected))
        .map(|(i, _)| s.env_name(i))
        .collect();
    let all_up = down.is_empty();
    let (status_label, status_color) = match (s.envs.len(), &s.status()) {
        (0 | 1, Status::Connected) => (
            s.env()
                .map_or("Connected".to_owned(), |e| e.label().to_owned()),
            look.muted,
        ),
        (0 | 1, Status::Connecting) => ("Connecting…".to_owned(), look.muted),
        (0 | 1, Status::Reconnecting { reason, .. }) => {
            (format!("Reconnecting: {reason}"), look.warning_fg)
        }
        (n, _) if all_up => (format!("{n} machines"), look.muted),
        (n, _) => (
            format!(
                "{}/{n} connected · {} offline",
                n - down.len(),
                down.join(", ")
            ),
            look.warning_fg,
        ),
    };
    let footer = row()
        .key("env-status")
        .w_full()
        .px(10.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(
            el().size(7.0, 7.0)
                .radius(3.5)
                .fill(if all_up && !s.envs.is_empty() {
                    look.accent
                } else {
                    look.warning_fg
                }),
        )
        .child(
            text(status_label)
                .font(k.ty.small)
                .color(status_color)
                .truncate()
                .grow(1.0),
        )
        .child(crate::actions::thread_menu(s, look, layer))
        .child(
            IconButton::new(&SETTINGS)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Settings")
                .shortcut("Ctrl ,")
                .expanded(s.settings_open)
                .on_press(|s: &mut App| s.toggle_settings())
                .build(k)
                .key("settings-toggle"),
        );
    let (key_rows, build_rows, kit) = (rows.clone(), rows.clone(), k.clone());
    let selected = s.selected.clone().map(|id| (s.focus, id));
    Sidebar::new()
        .label("Threads")
        .inset(0.0)
        .header_gap(4.0)
        .row_height(32.0)
        .header(new_row)
        .footer(footer)
        .virtual_rows(
            rows.len(),
            move |_: &App, i| match &key_rows[i] {
                Entry::Project { env, id, .. } => Cow::Owned(format!("p:{env}:{id}")),
                Entry::Thread { env, id, .. } => Cow::Owned(format!("t:{env}:{id}")),
                Entry::More { .. } => Cow::Borrowed("shelf:settled:more"),
                Entry::Shelf { shelf, .. } => Cow::Borrowed(match shelf {
                    Shelf::Pinned => "shelf:pinned",
                    Shelf::Snoozed => "shelf:snoozed",
                    Shelf::Settled => "shelf:settled",
                }),
            },
            move |_: &App, i| entry_row(&build_rows[i], selected.as_ref(), &kit),
        )
        .build(k)
}

fn entry_row(e: &Entry, selected: Option<&(usize, String)>, k: &Kit) -> El {
    match e {
        Entry::Project {
            env,
            id,
            title,
            count,
            collapsed,
        } => {
            let (env, toggle_id, new_id) = (*env, id.clone(), id.clone());
            SidebarHeader::new(title.clone())
                .count(*count)
                .collapsed(*collapsed)
                .on_toggle(move |s: &mut App, _| {
                    let key = folder_key(s, env, &toggle_id);
                    if !s.collapsed.remove(&key) {
                        s.collapsed.insert(key);
                    }
                })
                .action(
                    IconButton::new(&PLUS)
                        .variant(IconButtonVariant::GhostMuted)
                        .label("New thread in project")
                        .on_press(move |s: &mut App| s.new_thread_in(env, new_id.clone()))
                        .build(k)
                        .key("new-in-project"),
                )
                .build(k)
        }
        Entry::More { loading, remaining } => SidebarRow::new(if *loading {
            "Loading…".to_owned()
        } else {
            format!("Show more ({remaining})")
        })
        .height(32.0)
        .depth(1)
        .on_select(|s: &mut App, _| s.load_older_settled())
        .build(k),
        Entry::Shelf {
            shelf,
            count,
            expanded,
        } => {
            let shelf = *shelf;
            let header = SidebarHeader::new(match shelf {
                Shelf::Pinned => "Pinned",
                Shelf::Snoozed => "Snoozed",
                Shelf::Settled => "Settled",
            })
            .count(*count);
            match expanded {
                None => header.build(k),
                Some(open) => header
                    .collapsed(!open)
                    .on_toggle(move |s: &mut App, _| match shelf {
                        Shelf::Snoozed => s.prefs.toggle_snoozed(),
                        _ => s.prefs.toggle_settled(),
                    })
                    .build(k),
            }
        }
        Entry::Thread {
            env,
            id,
            title,
            status,
            age,
        } => {
            let (env, open) = (*env, id.clone());
            let mut r = SidebarRow::new(title.clone())
                .height(32.0)
                .depth(1)
                .detail(age.clone())
                .selected(selected.is_some_and(|(e, t)| *e == env && t == id))
                .on_select(move |s: &mut App, _| s.open_thread_in(env, open.clone()));
            let menu_id = id.clone();
            r = match status {
                Some(Status3::Busy) => r.status(RowStatus::Other(
                    rune::icon(&WORKING, 14.0).color(k.c.text_accent),
                )),
                Some(Status3::Attention) => r.status(RowStatus::Attention),
                Some(Status3::Unread) => r.status(RowStatus::Unread),
                None => r,
            };
            r.build(k).on_secondary_press(move |s: &mut App, at, _| {
                s.thread_menu = Some(crate::actions::ThreadMenu {
                    env,
                    thread: menu_id.clone(),
                    at,
                });
            })
        }
    }
}

fn theme_menu(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let trigger = IconButton::new(&PALETTE)
        .variant(IconButtonVariant::GhostMuted)
        .label("Theme")
        .expanded(s.popup == Some(Popup::Theme))
        .on_press(|s: &mut App| {
            s.popup = if s.popup == Some(Popup::Theme) {
                None
            } else {
                Some(Popup::Theme)
            }
        })
        .build(k)
        .key("theme-trigger");
    let current = s.prefs.theme.clone();
    let mut menu = Menu::new("theme-menu")
        .trigger(trigger)
        .open(s.popup == Some(Popup::Theme))
        .min_width(220.0)
        .on_close(|s: &mut App| s.popup = None);
    let env_ids: Vec<String> = s.env_themes.iter().map(|t| t.id.clone()).collect();
    let mut env_started = false;
    for (id, label) in theme::choices(&s.env_themes) {
        if env_ids.contains(&id) && !env_started {
            menu = menu.separator().group_label("From this machine");
            env_started = true;
        }
        let pick = id.clone();
        menu = menu.item(
            MenuItem::new(label)
                .checked(current == id)
                .on_select_tx(move |s: &mut App, tx| s.set_theme(&pick, tx)),
        );
    }
    menu.build(k, layer)
}

// ---------------------------------------------------------------------------------------------
// Timeline

/// Turn items the transcript shows; plumbing items (checkpoints, handoffs, forks) stay out.
fn shown(item: &TurnItem) -> bool {
    matches!(
        item.kind.as_str(),
        "user_message"
            | "assistant_message"
            | "reasoning"
            | "proposed_plan"
            | "todo_list"
            | "command_execution"
            | "file_change"
            | "file_search"
            | "web_search"
            | "dynamic_tool"
            | "subagent"
            | "approval_request"
            | "user_input_request"
            | "error"
            | "system_notice"
            | "run_interrupt_result"
            | "compaction"
            | "checkpoint"
    )
}

fn timeline(s: &App, look: &Look) -> El {
    let k = &look.kit;
    let body = match (&s.selected, &s.thread) {
        (None, _) => return empty_state(s, look),
        (Some(_), None) => col()
            .key("loading")
            .w_full()
            .h_full()
            .center()
            .child(Spinner::new().build(k)),
        (Some(_), Some(t)) => {
            // Only rows near the viewport are built, and a row whose item hasn't changed keeps
            // its subtree (`memo`), so a streaming update costs the streaming row, not the thread.
            // Queued messages live in the queue above the composer until they run.
            let queued: std::collections::HashSet<&str> = t
                .runs
                .iter()
                .filter(|r| r.status == "queued")
                .map(|r| r.id.as_str())
                .collect();
            let shown_rows: std::rc::Rc<Vec<usize>> = std::rc::Rc::new(
                (0..t.visible.len())
                    .filter(|&i| {
                        let item = &t.visible[i].item;
                        shown(item)
                            && !(item.kind == "user_message"
                                && item.run_id.as_deref().is_some_and(|r| queued.contains(r)))
                    })
                    .collect(),
            );
            let count = shown_rows.len() + usize::from(t.is_working());
            let (keys, rows) = (shown_rows.clone(), shown_rows);
            let row_look = look.clone();
            col()
                .key("rows")
                .w_full()
                .py(16.0)
                .growth(GrowthSpec::default())
                .virtual_rows(
                    count,
                    72.0,
                    move |s: &App, i| match keys
                        .get(i)
                        .and_then(|&j| s.thread.as_ref()?.visible.get(j))
                    {
                        Some(r) => Cow::Borrowed(r.source_item_id.as_str()),
                        None => Cow::Borrowed("working"),
                    },
                    move |s: &App, i| {
                        let look = &row_look;
                        let k = &look.kit;
                        let Some(r) = rows.get(i).and_then(|&j| s.thread.as_ref()?.visible.get(j))
                        else {
                            return row().key("working").w_full().justify_center().child(
                                row()
                                    .w_full()
                                    .max_w(COLUMN)
                                    .px(24.0)
                                    .py(10.0)
                                    .gap(8.0)
                                    .center_y()
                                    .child(rune::icon(&WORKING, 14.0).color(k.c.text_accent))
                                    .child(
                                        text(working_label(s)).font(k.ty.small).color(look.muted),
                                    ),
                            );
                        };
                        let item = &r.item;
                        let request = item.request_id.as_deref();
                        let pending = request
                            .and_then(|id| s.thread.as_ref()?.pending_request(id))
                            .is_some();
                        let answers = request.and_then(|id| s.answers.get(id)).map(|a| {
                            let mut v: Vec<_> = a.iter().collect();
                            v.sort();
                            format!("{v:?}")
                        });
                        // Everything the row reads that can change. The theme generation covers a
                        // published theme (Omarchy) changing colours under the same id.
                        let version = (
                            &item.updated_at,
                            &item.status,
                            item.text.as_ref().map(String::len),
                            pending,
                            answers,
                            s.expanded.contains(&item.id),
                            s.theme_generation,
                            look.dark,
                            s.outputs.get(&item.id).map(|o| {
                                (o.out.text.len(), o.running, o.loaded, o.error.is_some())
                            }),
                            s.thread.as_ref().map(|t| t.checkpoints.len()),
                        );
                        rune::memo::memo(r.source_item_id.clone(), version, || {
                            let inner = col()
                                .w_full()
                                .max_w(COLUMN)
                                .px(24.0)
                                .py(6.0)
                                .child(turn_item(s, look, item));
                            row()
                                .key(r.source_item_id.clone())
                                .w_full()
                                .justify_center()
                                .child(inner)
                                .streaming(item.is_streaming())
                        })
                    },
                )
        }
    };
    col()
        .key(format!("timeline:{}", s.selected.as_deref().unwrap_or("")))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec { stick_bottom: true })
        .scroll_anchor()
        .scroll_edge_fade(12.0)
        .child(body)
}

fn empty_state(s: &App, look: &Look) -> El {
    let k = &look.kit;
    let project = s
        .draft_project
        .as_deref()
        .and_then(|p| s.shell().project(p));
    let hint = match project {
        Some(p) => format!(
            "Start a thread in {}. Messages run on {}.",
            p.title,
            s.env().map_or("the server", |e| e.label())
        ),
        None if !s.shell().loaded => "Loading projects…".to_owned(),
        None => "Add a project from the web or desktop app to start a thread here.".to_owned(),
    };
    col()
        .key("empty")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .center()
        .gap(8.0)
        .child(
            text("What should we build?")
                .font(k.ty.body_medium)
                .color(look.text),
        )
        .child(
            text(hint)
                .font(k.ty.ui)
                .color(look.muted)
                .wrap()
                .max_w(420.0)
                .align(rune::Align::Center),
        )
}

fn turn_item(s: &App, look: &Look, item: &TurnItem) -> El {
    let k = &look.kit;
    let txt = item.text.as_deref().unwrap_or("");
    match item.kind.as_str() {
        "user_message" => {
            let bubble = col()
                .max_w(COLUMN * 0.8)
                .px(14.0)
                .py(10.0)
                .radius(k.radius.lg)
                .fill(look.message)
                .child(
                    text(txt.to_owned())
                        .font(k.ty.body)
                        .color(look.message_fg)
                        .wrap()
                        .selectable(true),
                );
            // How the message reached the agent, as the web's row marks it.
            let settled = matches!(
                item.status.as_str(),
                "completed" | "failed" | "cancelled" | "interrupted"
            );
            let label = match item.input_intent.as_deref() {
                Some("queued_turn") if !settled => Some("Queued"),
                Some("steer" | "promoted_queued_to_steer") => Some("Steer"),
                _ => None,
            };
            let mut c = col().w_full().items_end().gap(4.0).child(bubble);
            let files = item.attachments.as_deref().unwrap_or_default();
            if !files.is_empty() {
                let chips = files
                    .iter()
                    .fold(row().gap(6.0).flex_wrap().justify_end(), |r, f| {
                        let icon = if f["type"] == "image" {
                            &glyphs::FILE_IMAGE
                        } else {
                            &glyphs::FILE
                        };
                        r.child(
                            row()
                                .gap(4.0)
                                .px(8.0)
                                .py(3.0)
                                .center_y()
                                .radius(k.radius.md)
                                .border(1.0, look.border)
                                .child(rune::icon(icon, 12.0).color(look.muted))
                                .child(
                                    text(f["name"].as_str().unwrap_or("attachment").to_owned())
                                        .font(k.ty.small)
                                        .color(look.muted)
                                        .truncate(),
                                ),
                        )
                    });
                c = c.child(chips);
            }
            if let Some(label) = label {
                c = c.child(text(label).font(k.ty.small).color(look.muted));
            }
            c
        }
        "assistant_message" => Markdown::new(txt.to_owned())
            .streaming(item.is_streaming())
            .selectable(true)
            .build(k),
        "proposed_plan" => card(look).child(label_line(look, None, "Plan")).child(
            Markdown::new(item.markdown.clone().unwrap_or_default())
                .streaming(item.is_streaming())
                .selectable(true)
                .build(k),
        ),
        "reasoning" => {
            let first = txt
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or(if item.is_streaming() {
                    "Thinking…"
                } else {
                    "Thought"
                });
            let id = item.id.clone();
            let line = tool_line(
                look,
                None,
                "Thinking",
                Some(first.trim_matches('*').to_owned()),
                item,
            )
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut App| {
                if !s.expanded.remove(&id) {
                    s.expanded.insert(id.clone());
                }
            });
            if !s.expanded.contains(&item.id) {
                return line;
            }
            col().w_full().gap(6.0).child(line).child(
                col().pl(22.0).child(
                    text(txt.to_owned())
                        .font(k.ty.ui)
                        .color(look.muted)
                        .wrap()
                        .selectable(true),
                ),
            )
        }
        "todo_list" => {
            // As the web's: a quiet "Tasks 2/5 · current step" row that opens to every step.
            let steps = item.steps.as_deref().unwrap_or_default();
            let done = steps.iter().filter(|st| st.status == "completed").count();
            let current = steps
                .iter()
                .find(|st| st.status == "running")
                .or_else(|| steps.iter().find(|st| st.status == "pending"));
            let subject = match current {
                Some(st) => format!("{done}/{} · {}", steps.len(), st.text),
                None => format!("{done}/{}", steps.len()),
            };
            let id = item.id.clone();
            let line = tool_line(look, None, "Tasks", Some(subject), item)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut App| {
                    if !s.expanded.remove(&id) {
                        s.expanded.insert(id.clone());
                    }
                });
            if !s.expanded.contains(&item.id) {
                return line;
            }
            let mut list = col().pl(22.0).gap(4.0);
            if let Some(why) = item.explanation.as_deref().filter(|e| !e.is_empty()) {
                list = list.child(text(why.to_owned()).font(k.ty.ui).color(look.muted).wrap());
            }
            for st in steps {
                let (mark, color) = match st.status.as_str() {
                    "completed" => ("✓", look.muted),
                    "running" => ("●", look.accent),
                    _ => ("○", look.muted),
                };
                list = list.child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .child(text(mark.to_owned()).font(k.ty.ui).color(color))
                        .child(
                            text(st.text.clone())
                                .font(k.ty.ui)
                                .color(if st.status == "completed" {
                                    look.muted
                                } else {
                                    look.text
                                })
                                .wrap()
                                .shrink(),
                        ),
                );
            }
            col().w_full().gap(6.0).child(line).child(list)
        }
        "command_execution" => {
            let cmd = item
                .input_text()
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
                .to_owned();
            let failed = item.exit_code.is_some_and(|c| c != 0);
            let mut line = row()
                .w_full()
                .px(10.0)
                .py(7.0)
                .gap(8.0)
                .center_y()
                .radius(k.radius.md)
                .fill(look.code_bg)
                .child(rune::icon(&TERMINAL, 14.0).color(look.muted))
                .child(
                    text(cmd)
                        .font(k.ty.mono)
                        .color(look.code_fg)
                        .truncate()
                        .grow(1.0),
                );
            line = match (item.status.as_str(), item.exit_code) {
                ("running" | "pending", _) => {
                    line.child(rune::icon(&WORKING, 12.0).color(k.c.text_accent))
                }
                (_, Some(code)) if failed => line.child(
                    text(format!("exit {code}"))
                        .font(k.ty.small)
                        .color(look.error_fg),
                ),
                _ => line,
            };
            let id = item.id.clone();
            let line = line
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut App| s.toggle_output(&id));
            match s.outputs.get(&item.id) {
                None => line,
                Some(o) => col()
                    .w_full()
                    .gap(4.0)
                    .child(line)
                    .child(command_output(o, look)),
            }
        }
        "checkpoint" => {
            // A turn's changed-files summary; clicking it opens that turn in the diff panel.
            let Some(c) = item
                .checkpoint_id
                .as_deref()
                .and_then(|id| s.thread.as_ref()?.checkpoints.iter().find(|c| c.id == id))
            else {
                return col();
            };
            let (Some(turn), false) = (c.app_run_ordinal, c.files.is_empty()) else {
                return col();
            };
            let (adds, dels) = c
                .files
                .iter()
                .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
            let files = if c.files.len() == 1 {
                "1 file changed".to_owned()
            } else {
                format!("{} files changed", c.files.len())
            };
            row()
                .w_full()
                .gap(8.0)
                .center_y()
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut App| s.open_diff(crate::DiffSel::Turn(turn)))
                .child(rune::icon(&DIFF, 14.0).color(look.muted))
                .child(text(files).font(k.ty.small_medium).color(look.muted))
                .child(
                    text(format!("+{adds}"))
                        .font(k.ty.small)
                        .color(k.c.success.icon),
                )
                .child(
                    text(format!("−{dels}"))
                        .font(k.ty.small)
                        .color(look.error_fg),
                )
                .child(text("View changes").font(k.ty.small).color(k.c.text_accent))
        }
        "file_change" => {
            let name = item.file_name.clone().unwrap_or_default();
            let stats = match (item.additions, item.deletions) {
                (None, None) => None,
                (a, d) => Some(format!("+{} −{}", a.unwrap_or(0), d.unwrap_or(0))),
            };
            tool_line(look, Some(&PENCIL), "Edited", Some(name), item)
                .child(stats.map_or(el(), |s| text(s).font(k.ty.small).color(look.muted)))
        }
        "file_search" => tool_line(
            look,
            Some(&glyphs::SEARCH),
            "Searched",
            item.pattern.clone(),
            item,
        ),
        "web_search" => tool_line(
            look,
            Some(&glyphs::SEARCH),
            "Searched the web",
            item.patterns.as_ref().map(|p| p.join(", ")),
            item,
        ),
        "dynamic_tool" => tool_line(
            look,
            None,
            item.tool_name.as_deref().unwrap_or("Tool"),
            item.title.clone(),
            item,
        ),
        "subagent" => tool_line(
            look,
            Some(&glyphs::SPARKLE),
            "Subagent",
            item.title
                .clone()
                .or(item.prompt.clone())
                .map(|p| p.lines().next().unwrap_or("").to_owned()),
            item,
        ),
        "compaction" => tool_line(look, None, "Compacted context", None, item),
        "approval_request" => approval(s, look, item),
        "user_input_request" => question(s, look, item),
        "error" => {
            let msg = item
                .failure
                .as_ref()
                .map_or("The provider reported an error", |f| f.message.as_str())
                .to_owned();
            col()
                .w_full()
                .px(12.0)
                .py(10.0)
                .radius(k.radius.md)
                .fill(look.error_surface)
                .child(
                    text(msg)
                        .font(k.ty.ui)
                        .color(look.error_fg)
                        .wrap()
                        .selectable(true),
                )
        }
        _ => text(item.message.clone().unwrap_or_default())
            .font(k.ty.small)
            .color(look.muted)
            .wrap(),
    }
}

fn card(look: &Look) -> El {
    col()
        .w_full()
        .padding(14.0)
        .gap(10.0)
        .radius(look.kit.radius.lg)
        .fill(look.surface)
        .border(1.0, look.border)
}

fn label_line(look: &Look, icon: Option<&'static Icon>, label: &str) -> El {
    let k = &look.kit;
    let mut r = row().gap(6.0).center_y();
    if let Some(i) = icon {
        r = r.child(rune::icon(i, 13.0).color(look.muted));
    }
    r.child(
        text(label.to_owned())
            .font(k.ty.small_medium)
            .color(look.muted),
    )
}

/// One quiet line for a tool call: verb, subject, and a spinner while it runs.
fn tool_line(
    look: &Look,
    icon: Option<&'static Icon>,
    verb: &str,
    subject: Option<String>,
    item: &TurnItem,
) -> El {
    let k = &look.kit;
    let mut r = row()
        .w_full()
        .gap(8.0)
        .py(2.0)
        .center_y()
        .child(label_line(look, icon, verb));
    if let Some(s) = subject.filter(|s| !s.is_empty()) {
        r = r.child(
            text(s)
                .font(k.ty.small)
                .color(look.muted)
                .truncate()
                .grow(1.0),
        );
    } else {
        r = r.child(el().grow(1.0));
    }
    if matches!(item.status.as_str(), "running" | "pending") || item.is_streaming() {
        r = r.child(rune::icon(&WORKING, 11.0).color(k.c.text_accent));
    }
    r
}

/// The command palette over a dimmed window: a search field and grouped rows.
fn palette(s: &App, look: &Look) -> El {
    let k = &look.kit;
    let Some(p) = &s.palette else { return col() };
    let items = s.palette_items();
    let field = rune::text_input(&p.query, rune::TextStyle { ..k.ty.body })
        .key("palette-input")
        .w_full()
        .color(look.text)
        .placeholder("Search threads, actions and themes", k.c.text_faint)
        .caret(look.text, k.selection())
        .label("Command palette")
        .on_edit(|s: &mut App| &mut s.palette.as_mut().expect("palette open").query)
        .on_input_change(|s: &mut App, _| {
            if let Some(p) = s.palette.as_mut() {
                p.active = 0;
            }
        })
        .on_input_key(|s: &mut App, ev: &rune::KeyEvent, tx| s.palette_key(&ev.key, tx));
    let mut list = col().key("palette-rows").w_full().gap(2.0);
    let mut group = "";
    for (i, item) in items.iter().enumerate() {
        if item.group != group {
            group = item.group;
            list = list.child(
                text(group)
                    .font(k.ty.small)
                    .color(look.muted)
                    .px(10.0)
                    .pt(8.0)
                    .pb(2.0),
            );
        }
        let active = i == p.active;
        let mut r = row()
            .key(format!("palette-{i}"))
            .w_full()
            .px(10.0)
            .py(7.0)
            .gap(10.0)
            .center_y()
            .radius(k.radius.md)
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |s: &mut App, tx| s.run_palette(i, tx))
            .child(
                text(item.label.clone())
                    .font(k.ty.ui)
                    .color(look.text)
                    .truncate()
                    .grow(1.0),
            );
        if active {
            r = r.fill(k.c.bg_accent.alpha(0.35));
        }
        if let Some(d) = &item.detail {
            r = r.child(text(d.clone()).font(k.ty.small).color(look.muted));
        }
        if let Some(sc) = item.shortcut {
            r = r.child(text(sc).font(k.ty.small).color(look.muted));
        }
        list = list.child(r);
    }
    if items.is_empty() {
        list = list.child(
            text("No matches")
                .font(k.ty.ui)
                .color(look.muted)
                .padding(12.0),
        );
    }
    let card = col()
        .key("palette-card")
        .w_full()
        .max_w(620.0)
        .max_h(520.0)
        .radius(k.radius.lg)
        .fill(look.raised)
        .border(1.0, look.border)
        .shadow(12.0, 40.0, rune::Color::BLACK.alpha(0.35))
        .child(row().w_full().px(14.0).py(12.0).child(field))
        .child(col().w_full().h(1.0).fill(look.border))
        .child(
            col()
                .w_full()
                .shrink()
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(col().w_full().padding(6.0).child(list)),
        );
    col()
        .key("palette-overlay")
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(rune::Color::BLACK.alpha(0.25))
        .child(rune_kit::scrim("palette-scrim", |s: &mut App| {
            s.close_palette()
        }))
        .child(
            row()
                .w_full()
                .justify_center()
                .pt(90.0)
                .px(16.0)
                .child(card),
        )
}

/// The inspector beside the thread: Changes or Files, picked by a tab row.
fn inspector(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let tab = s.inspector;
    let tabs = rune_kit::SegmentedControl::new(
        &["Changes", "Files"],
        Some(usize::from(tab == Inspector::Files)),
    )
    .label("Inspector")
    .fit()
    .on_pick(|s: &mut App, i, _: &mut rune::Tx| {
        s.show_inspector(if i == 1 {
            Inspector::Files
        } else {
            Inspector::Changes
        })
    })
    .build(k)
    .key("inspector-tabs");
    let body = match tab {
        Inspector::Changes => diff_panel(s, look, layer),
        Inspector::Files => files_panel(s, look),
    };
    col()
        .key("inspector")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(row().w_full().px(12.0).pt(8.0).child(tabs))
        .child(col().w_full().grow(1.0).min_h(0.0).child(body))
}

/// The Files tab: the workspace tree, or the open file's source with a way back to it.
fn files_panel(s: &App, look: &Look) -> El {
    let k = &look.kit;
    let note = |t: String| {
        col()
            .w_full()
            .grow(1.0)
            .center()
            .padding(16.0)
            .child(text(t).font(k.ty.ui).color(look.muted).wrap())
    };
    if s.files.cwd.is_none() {
        return note("Open a thread to browse its files".into());
    }
    if let Some(f) = &s.files.open {
        return file_view(f, look);
    }
    if let Some(e) = s.files.errors.get("") {
        return note(format!("Couldn't list the files: {e}"));
    }
    if !s.files.root_loaded() {
        return note("Loading files…".into());
    }
    if s.files.tree.is_empty() {
        return note("This folder is empty".into());
    }
    let (muted, faint) = (look.muted, look.muted.alpha(0.6));
    let ignored = s.files.ignored.clone();
    let tree = rune_kit::Tree::new(
        |s: &App| &s.files.tree,
        |s: &mut App| &mut s.files.tree,
        rune_kit::FileNode::label,
    )
    .name("Files")
    .row(move |n, r| {
        if crate::files::is_placeholder(r.id) {
            return rune_kit::TreeItem::new(n.name.clone()).label_color(muted);
        }
        let item = rune_kit::TreeItem::file(n, r);
        if ignored.contains(&n.path) {
            item.label_color(faint).icon_color(faint)
        } else {
            item
        }
    })
    .on_select(|s: &mut App, id, _| {
        if s.files.tree.get(id).is_some_and(|n| !n.dir) {
            s.open_file(id);
        }
    })
    .build(s, k);
    col()
        .key("files-panel")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(8.0)
        .py(8.0)
        .child(tree.grow(1.0).min_h(0.0))
}

const SOURCE_LINE: f32 = 20.0;

/// A file's source: a back button and its path, then every line numbered and highlighted. Rows
/// are virtual, so a 20,000-line file costs what the visible lines do.
fn file_view(f: &crate::files::OpenFile, look: &Look) -> El {
    let k = &look.kit;
    let back = IconButton::new(&BACK)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label("Back to files")
        .shortcut("Esc")
        .on_press(|s: &mut App| s.close_file())
        .build(k)
        .key("file-back");
    let (dir, name) = f.path.rsplit_once('/').unwrap_or(("", &f.path));
    let path = row()
        .gap(6.0)
        .center_y()
        .min_w(0.0)
        .shrink()
        .child(
            text(name.to_owned())
                .font(k.ty.ui)
                .color(look.text)
                .truncate(),
        )
        .child(
            text(dir.to_owned())
                .font(k.ty.small)
                .color(look.muted)
                .truncate()
                .shrink(),
        );
    let mut header = row()
        .w_full()
        .px(8.0)
        .py(6.0)
        .gap(4.0)
        .center_y()
        .child(back)
        .child(path);
    if f.is_markdown() {
        let source = f.source;
        header = header.child(el().grow(1.0)).child(
            Button::new(if source { "Preview" } else { "Source" })
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| {
                    if let Some(f) = s.files.open.as_mut() {
                        f.source = !source;
                    }
                })
                .build(k)
                .key("file-source"),
        );
    }
    let note = |t: String, c| {
        col()
            .w_full()
            .grow(1.0)
            .center()
            .padding(16.0)
            .child(text(t).font(k.ty.ui).color(c).wrap())
    };
    let body = if let Some(e) = &f.error {
        note(format!("Couldn't open the file: {e}"), look.error_fg)
    } else if f.loading {
        note("Loading…".into(), look.muted)
    } else if f.is_markdown() && !f.source {
        col()
            .key("file-markdown")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .w_full()
                    .px(16.0)
                    .py(12.0)
                    .child(Markdown::new(f.text.to_string()).selectable(true).build(k)),
            )
    } else {
        source_lines(f, look)
    };
    let mut panel = col()
        .key("file-view")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(header)
        .child(body);
    if f.truncated {
        panel = panel.child(
            row().w_full().px(12.0).py(6.0).child(
                text(format!(
                    "Showing the start of a {} file",
                    byte_size(f.byte_length)
                ))
                .font(k.ty.small)
                .color(look.muted),
            ),
        );
    }
    panel
}

fn source_lines(f: &crate::files::OpenFile, look: &Look) -> El {
    let k = &look.kit;
    let syntax = rune_kit::syntax::SyntaxTheme::from_kit(k);
    let (text_src, lines, highlights) = (f.text.clone(), f.lines.clone(), f.highlights.clone());
    let digits = lines.len().to_string().len() as f32;
    let gutter = 12.0 + digits * 7.5;
    let (fg, faint, mono) = (look.code_fg, look.muted.alpha(0.7), k.ty.mono);
    let rows = col()
        .key("file-lines")
        .w_full()
        .py(4.0)
        .growth(GrowthSpec::default())
        .virtual_rows(
            lines.len(),
            SOURCE_LINE,
            |_: &App, i| Cow::Owned(i.to_string()),
            move |_: &App, i| {
                let (a, b) = lines[i];
                let line = text_src[a..b].trim_end_matches('\r').to_owned();
                // Highlights are sorted and disjoint: start at the first that ends past `a`.
                let first = highlights.partition_point(|(r, _)| r.end <= a);
                let mut spans = Vec::new();
                for (r, paint) in &highlights[first..] {
                    if r.start >= b {
                        break;
                    }
                    let (s0, s1) = (r.start.max(a) - a, r.end.min(b) - a);
                    if s0 < s1 && s1 <= line.len() {
                        let mut span = rune::Span::new(s0, s1);
                        span.color = Some(syntax.paint(*paint));
                        spans.push(span);
                    }
                }
                row()
                    .w_full()
                    .h(SOURCE_LINE)
                    .center_y()
                    .child(
                        text((i + 1).to_string())
                            .font(mono)
                            .color(faint)
                            .w(gutter)
                            .pr(12.0)
                            .align(rune::Align::End),
                    )
                    .child(
                        rune::rich(line, spans)
                            .font(mono)
                            .color(fg)
                            .truncate()
                            .shrink(),
                    )
            },
        );
    col()
        .key("file-scroll")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .fill(look.code_bg)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(rows)
}

fn byte_size(n: u64) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1 << 20) as f64),
        n if n >= 1 << 10 => format!("{} KB", n >> 10),
        n => format!("{n} B"),
    }
}

/// The diff inspector: a turn picker, unified/split, and rune-kit's diff view of the patch.
fn diff_panel(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let turns = s
        .thread
        .as_ref()
        .map(|t| t.diff_turns())
        .unwrap_or_default();
    // (label, selection); labels are unique, so a pick maps back by label.
    let mut options: Vec<(String, crate::DiffSel)> = vec![
        ("Latest turn".into(), crate::DiffSel::Latest),
        ("All turns".into(), crate::DiffSel::All),
    ];
    for c in &turns {
        let n = c.app_run_ordinal.unwrap_or(0);
        let files = c.files.len();
        options.push((
            format!(
                "Turn {n} · {files} file{}",
                if files == 1 { "" } else { "s" }
            ),
            crate::DiffSel::Turn(n),
        ));
    }
    let current = options
        .iter()
        .find(|(_, sel)| *sel == s.diff.sel)
        .map(|(l, _)| l.clone());
    let labels: Vec<String> = options.iter().map(|(l, _)| l.clone()).collect();
    let picker = rune_kit::Select::new("diff-turn", &labels)
        .current(current.as_deref())
        .open(s.popup == Some(Popup::DiffTurn))
        .on_toggle(|s: &mut App| {
            s.popup = if s.popup == Some(Popup::DiffTurn) {
                None
            } else {
                Some(Popup::DiffTurn)
            }
        })
        .on_close(|s: &mut App| s.popup = None)
        .on_choose(move |s: &mut App, label: &str, _: &mut rune::Tx| {
            if let Some((_, sel)) = options.iter().find(|(l, _)| l == label) {
                s.open_diff(*sel);
            }
        })
        .build(k, layer)
        .key("diff-turn-picker");
    let split = s.diff.state.mode() == rune_kit::diff::DiffMode::Split;
    let mode = Button::new(if split { "Unified" } else { "Split" })
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(move |s: &mut App| {
            let next = if split {
                rune_kit::diff::DiffMode::Unified
            } else {
                rune_kit::diff::DiffMode::Split
            };
            s.diff.state.set_mode(next);
        })
        .build(k)
        .key("diff-mode");
    let header = row()
        .w_full()
        .px(12.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .child(picker)
        .child(el().grow(1.0))
        .child(mode);
    let note = |t: &str| {
        col()
            .w_full()
            .grow(1.0)
            .center()
            .child(text(t.to_owned()).font(k.ty.ui).color(look.muted))
    };
    let body = if let Some(e) = &s.diff.error {
        note(&format!("Couldn't load the diff: {e}"))
    } else if turns.is_empty() {
        note("No changes in this thread yet")
    } else if s.diff.loading && s.diff.state.files().is_empty() {
        note("Loading changes…")
    } else if s.diff.state.files().is_empty() {
        note("No changes in this turn")
    } else {
        rune_kit::diff::DiffView::new(|s: &App| &s.diff.state, |s: &mut App| &mut s.diff.state)
            .key("diff-view")
            .build(s, k)
    };
    col()
        .key("diff-panel")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(header)
        .child(col().w_full().grow(1.0).min_h(0.0).child(body))
}

/// An expanded command's output: styled like the web's panel, scrolled to its end while it runs.
fn command_output(o: &crate::CommandOutput, look: &Look) -> El {
    let k = &look.kit;
    let body = if let Some(e) = &o.error {
        text(format!("Couldn't load the output: {e}"))
            .font(k.ty.small)
            .color(look.error_fg)
            .wrap()
    } else if !o.loaded {
        text("Loading output…").font(k.ty.small).color(look.muted)
    } else if o.out.text.is_empty() {
        text(if o.running {
            "No output yet"
        } else {
            "No output"
        })
        .font(k.ty.small)
        .color(look.muted)
    } else {
        let palette = if look.dark {
            &t3_client::terminal::PALETTE_DARK
        } else {
            &t3_client::terminal::PALETTE_LIGHT
        };
        let mut plain = String::new();
        let mut spans = Vec::new();
        for (chunk, style) in t3_client::terminal::spans(&o.out.text, palette, 2_000) {
            let start = plain.len();
            plain.push_str(&chunk);
            let Some(st) = style else { continue };
            let rgb = |c: [u8; 3]| {
                crate::theme::lin(t3_theme::Srgb {
                    r: c[0] as f32 / 255.0,
                    g: c[1] as f32 / 255.0,
                    b: c[2] as f32 / 255.0,
                    a: 1.0,
                })
            };
            let mut span = rune::Span::new(start, plain.len());
            if let Some(c) = st.color {
                span.color = Some(if st.dim { rgb(c).alpha(0.65) } else { rgb(c) });
            } else if st.dim {
                span.color = Some(look.code_fg.alpha(0.65));
            }
            span.bg = st.background.map(rgb);
            if st.bold {
                span = span.weight(600.0);
            }
            span.italic = st.italic;
            if st.underline {
                span.underline = Some(span.color.unwrap_or(look.code_fg));
            }
            spans.push(span);
        }
        rune::rich(plain, spans)
            .font(k.ty.mono)
            .color(look.code_fg)
            .wrap_anywhere()
            .selectable(true)
    };
    let mut panel = col().w_full().gap(6.0).child(body);
    if o.out.truncated {
        panel = panel.child(
            text("Showing the last 64 KB")
                .font(k.ty.small)
                .color(look.muted),
        );
    }
    col()
        .w_full()
        .max_h(320.0)
        .radius(k.radius.md)
        .fill(look.code_bg)
        .scroll(ScrollSpec {
            stick_bottom: o.running,
        })
        .child(col().w_full().padding(10.0).child(panel))
}

/// "Working for 1m 4s", from the run's start; the clock ticks once a second while it works.
fn working_label(s: &App) -> String {
    match s
        .thread
        .as_ref()
        .and_then(|t| t.thread.activity_run_started_at.as_deref())
        .and_then(parse_iso)
    {
        Some(start) => format!("Working for {}", elapsed(s.clock - start)),
        None => "Working".to_owned(),
    }
}

/// "12s", "3m 4s", "1h 2m".
fn elapsed(secs: i64) -> String {
    let secs = secs.max(0);
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
    }
}

fn approval(s: &App, look: &Look, item: &TurnItem) -> El {
    let k = &look.kit;
    let request_id = item.request_id.clone().unwrap_or_default();
    let pending = s
        .thread
        .as_ref()
        .and_then(|t| t.pending_request(&request_id))
        .is_some();
    let what = match item.request_kind.as_deref() {
        Some("command") => "Run a command",
        Some("file-change") => "Change files",
        Some("file-read") => "Read files",
        Some("permission") => "Grant a permission",
        _ => "Approve an action",
    };
    let mut c = card(look)
        .child(label_line(
            look,
            None,
            if pending {
                "Needs your approval"
            } else {
                "Approval"
            },
        ))
        .child(text(what.to_owned()).font(k.ty.ui_medium).color(look.text));
    for body in [item.prompt.as_ref(), item.reason.as_ref()]
        .into_iter()
        .flatten()
    {
        c = c.child(
            text(body.clone())
                .font(k.ty.mono)
                .color(look.code_fg)
                .wrap()
                .selectable(true),
        );
    }
    if !pending {
        return c;
    }
    let options: Vec<(String, String)> = match &item.options {
        Some(o) if !o.is_empty() => o
            .iter()
            .map(|o| (o.decision.clone(), o.label.clone()))
            .collect(),
        _ => vec![
            ("accept".into(), "Approve".into()),
            ("acceptForSession".into(), "Approve for session".into()),
            ("decline".into(), "Decline".into()),
        ],
    };
    let buttons = options.into_iter().enumerate().fold(
        row().gap(8.0).flex_wrap(),
        |r, (i, (decision, label))| {
            let rid = request_id.clone();
            let variant = if i == 0 {
                ButtonVariant::Contrast
            } else if matches!(decision.as_str(), "decline" | "cancel") {
                ButtonVariant::Ghost
            } else {
                ButtonVariant::Neutral
            };
            r.child(
                Button::new(label)
                    .variant(variant)
                    .on_press(move |s: &mut App| s.respond(&rid, &decision))
                    .build(k)
                    .key(format!("decide-{i}")),
            )
        },
    );
    c.child(buttons)
}

fn question(s: &App, look: &Look, item: &TurnItem) -> El {
    let k = &look.kit;
    let request_id = item.request_id.clone().unwrap_or_default();
    let pending = s
        .thread
        .as_ref()
        .and_then(|t| t.pending_request(&request_id))
        .is_some();
    let picks = s.answers.get(&request_id);
    let mut c = card(look).child(label_line(
        look,
        None,
        if pending {
            "Question for you"
        } else {
            "Question"
        },
    ));
    for q in item.questions.iter().flatten() {
        c = c.child(
            text(q.question.clone())
                .font(k.ty.ui_medium)
                .color(look.text)
                .wrap(),
        );
        if !pending {
            continue;
        }
        let multi = q.multi_select == Some(true);
        let chosen = picks.and_then(|p| p.get(&q.id));
        let opts = q
            .options
            .iter()
            .enumerate()
            .fold(col().gap(6.0), |c, (i, o)| {
                let value = o.value.clone().unwrap_or_else(|| o.label.clone());
                let on = chosen.is_some_and(|v| v.contains(&value));
                let (rid, qid) = (request_id.clone(), q.id.clone());
                let button = Button::new(o.label.clone())
                    .variant(if on {
                        ButtonVariant::Contrast
                    } else {
                        ButtonVariant::Outline
                    })
                    .on_press(move |s: &mut App| {
                        let v = s
                            .answers
                            .entry(rid.clone())
                            .or_default()
                            .entry(qid.clone())
                            .or_default();
                        if let Some(at) = v.iter().position(|x| *x == value) {
                            v.remove(at);
                        } else {
                            if !multi {
                                v.clear();
                            }
                            v.push(value.clone());
                        }
                    })
                    .build(k)
                    .key(format!("q-{}-{i}", q.id));
                let line = row().gap(10.0).center_y().child(button);
                c.child(if o.description.is_empty() {
                    line
                } else {
                    line.child(
                        text(o.description.clone())
                            .font(k.ty.small)
                            .color(look.muted)
                            .wrap()
                            .grow(1.0),
                    )
                })
            });
        c = c.child(opts);
    }
    if pending {
        let rid = request_id.clone();
        c = c.child(
            row().justify_end().child(
                Button::new("Send answers")
                    .variant(ButtonVariant::Contrast)
                    .on_press(move |s: &mut App| s.answer(&rid))
                    .build(k)
                    .key("answer"),
            ),
        );
    }
    c
}

// ---------------------------------------------------------------------------------------------
// Composer

fn composer(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let working = s.thread.as_ref().is_some_and(|t| t.is_working());
    let ready_plan = s.ready_plan().is_some();
    let placeholder = if s.editing_queued.is_some() {
        "Rewrite the queued message: Return saves it, Esc cancels…"
    } else if ready_plan && !working {
        "Return implements the plan, or type to refine it…"
    } else if working {
        "Steer the agent…"
    } else if s.selected.is_none() {
        "Describe a task…"
    } else {
        "Ask for follow-up changes…"
    };
    // The editor sits straight on the composer surface, as the web's does: kit's TextArea
    // would draw a second bordered box inside it. Return sends, Shift+Return breaks a line.
    let style = rune::TextStyle {
        line_height: (k.ty.body.size * 1.5).round(),
        ..k.ty.body
    };
    let editor = rune::text_input(&s.composer, style)
        .key("composer-input")
        .w_full()
        .color(look.text)
        .placeholder(placeholder, k.c.text_faint)
        .caret(look.text, k.selection())
        .label("Message")
        .on_edit(|s: &mut App| &mut s.composer)
        .on_submit(|s: &mut App, tx| s.send(false, tx))
        // Ctrl/Cmd+Return sends the alternate follow-up (queue while the agent works).
        .on_input_change(|s: &mut App, _| s.refresh_menu())
        // A pasted screenshot or copied files attach; text pastes as text.
        .on_input_paste(|s: &mut App, content: &rune::ClipboardContent, tx| {
            if let Some(img) = content.image() {
                s.attach_bytes("pasted-image.png".into(), img.png().to_vec(), tx);
                return true;
            }
            if !content.files().is_empty() {
                let files = content.files().to_vec();
                s.attach_paths(&files, tx);
                return true;
            }
            false
        })
        .on_input_key(|s: &mut App, ev: &rune::KeyEvent, tx| {
            if !ev.mods.cmd && !ev.mods.ctrl && s.menu_key(&ev.key) {
                return true;
            }
            let alternate = matches!(ev.key, rune::Key::Enter)
                && (ev.mods.cmd || ev.mods.ctrl)
                && !ev.mods.shift;
            if alternate {
                s.send(true, tx);
            }
            alternate
        });
    let mut field = col()
        .key("composer-field")
        .w_full()
        .gap(8.0)
        .px(4.0)
        .pt(2.0);
    if let Some(queue) = crate::queue::panel(s, look) {
        field = field.child(queue);
    }
    if let Some(banner) = crate::plan::banner(s, look) {
        field = field.child(banner);
    }
    if let Some(chips) = crate::attach::chips(s, look) {
        field = field.child(chips);
    }
    let field = field.child(rune::auto_grow(editor, 2, 12));
    let attach = IconButton::new(&glyphs::FILE)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label("Attach files")
        .help("Or paste a screenshot or copied files")
        .on_press_tx(|s: &mut App, tx| s.pick_attachment(tx))
        .build(k)
        .key("attach");
    let send = if working && s.composer.text.trim().is_empty() {
        IconButton::new(&STOP)
            .variant(IconButtonVariant::Contrast)
            .label("Stop")
            .shortcut("Ctrl .")
            .on_press(|s: &mut App| s.stop())
            .build(k)
            .key("stop")
    } else {
        let mut b = IconButton::new(&SEND)
            .variant(IconButtonVariant::Submit)
            .label(if working { "Steer" } else { "Send" })
            .on_press_tx(|s: &mut App, tx| s.send(tx.mods().cmd || tx.mods().ctrl, tx));
        if working {
            b = b.help("Return steers the agent. Ctrl+Return or Ctrl-click queues the message until it finishes.");
        }
        b.build(k).key("send")
    };
    // Wraps to a second line rather than clipping in a narrow tiled window.
    let mut controls = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(attach)
        .child(model_menu(s, look, layer))
        .child(
            Button::new(if s.plan_mode() { "Plan mode" } else { "Plan" })
                .variant(if s.plan_mode() {
                    ButtonVariant::Neutral
                } else {
                    ButtonVariant::Ghost
                })
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.toggle_plan_mode())
                .build(k)
                .key("plan-toggle"),
        );
    if s.selected.is_none() {
        controls = controls.child(project_menu(s, look, layer));
        controls = controls.children(crate::workspace::controls(s, look, layer));
    } else if let Some(branch) = crate::workspace::thread_branch(s, look) {
        controls = controls.child(branch);
    }
    if let Status::Reconnecting { .. } = &s.status() {
        controls = controls.child(
            text("Offline — messages can't send")
                .font(k.ty.small)
                .color(look.warning_fg),
        );
    }
    if let Some(menu) = &s.menu {
        let items = menu.items.iter().map(|it| {
            let item = rune_kit::PopupItem::new(it.id.clone(), it.label.clone());
            match &it.detail {
                Some(d) => item.description(d.clone()),
                None => item,
            }
        });
        rune_kit::PopupList::<App>::new("composer-menu", "composer")
            .anchored("composer", rune::Side::Top, rune::Align::Start, 6.0)
            .width(440.0)
            .max_height(320.0)
            .open(true)
            .items(items)
            .active(Some(menu.active))
            .empty_message(if menu.loading {
                "Searching…"
            } else {
                "No matches"
            })
            .on_active(|s: &mut App, i| {
                if let Some(m) = s.menu.as_mut() {
                    m.active = i;
                }
            })
            .on_choose(|s: &mut App, id: &str, _| s.pick(id))
            .on_close(|s: &mut App| s.menu = None)
            .build(k, layer);
    }
    let surface = col()
        .key("composer")
        .w_full()
        .max_w(COLUMN)
        .padding(10.0)
        .gap(8.0)
        .radius(k.radius.lg)
        .fill(look.raised)
        .border(1.0, look.border)
        .child(field)
        .child(controls.child(el().grow(1.0)).child(send))
        .on_drop_files(|s: &mut App, paths: &[std::path::PathBuf], tx| s.attach_paths(paths, tx));
    row()
        .key("composer-dock")
        .w_full()
        .justify_center()
        .px(24.0)
        .pt(4.0)
        .pb(16.0)
        .child(surface)
}

/// The provider model a selection names, if this server offers it.
fn selected_model<'a>(
    s: &'a App,
    m: &ModelSelection,
) -> Option<&'a t3_client::protocol::ServerProviderModel> {
    s.config()?
        .providers
        .iter()
        .find(|p| p.instance_id == m.instance_id)?
        .models
        .iter()
        .find(|x| x.slug == m.model)
}

/// "Opus 5.5", plus the reasoning level when it isn't the model's default ("Opus 5.5 High").
fn model_label(s: &App, m: &ModelSelection) -> String {
    let model = selected_model(s, m);
    let mut label = model
        .map(|x| x.short_name.clone().unwrap_or_else(|| x.name.clone()))
        .unwrap_or_else(|| m.model.clone());
    let effort = model.and_then(|x| x.option_descriptors().iter().find(|d| d.kind == "select"));
    if let Some(d) = effort {
        let value = m
            .option(&d.id)
            .cloned()
            .unwrap_or_else(|| d.default_value());
        if value != d.default_value() {
            if let Some(choice) = d.options.iter().find(|o| value == o.id.as_str()) {
                label = format!("{label} {}", choice.label);
            }
        }
    }
    label
}

fn model_menu(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let current = s.effective_model();
    let label = current
        .as_ref()
        .map_or("No model".to_owned(), |m| model_label(s, m));
    let open = s.popup == Some(Popup::Model);
    let trigger = Button::new(label)
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.popup = if open { None } else { Some(Popup::Model) })
        .build(k)
        .key("model-trigger");
    let mut menu = Menu::new("model-menu")
        .trigger(trigger)
        .open(open)
        .min_width(240.0)
        .on_close(|s: &mut App| s.popup = None);
    // The current model's settings first (reasoning, service tier, fast mode), then the models.
    let descriptors = current
        .as_ref()
        .and_then(|m| selected_model(s, m))
        .map(|m| m.option_descriptors().to_vec())
        .unwrap_or_default();
    if let Some(sel) = current.clone().filter(|_| !descriptors.is_empty()) {
        let mut after_list = false;
        for d in descriptors {
            let value = sel
                .option(&d.id)
                .cloned()
                .unwrap_or_else(|| d.default_value());
            // Switches after a list would read as more of its choices.
            if after_list {
                menu = menu.separator();
            }
            after_list = d.kind != "boolean";
            if d.kind == "boolean" {
                let on = value == serde_json::Value::Bool(true);
                let (sel, id) = (sel.clone(), d.id.clone());
                menu = menu.item(MenuItem::new(d.label.clone()).checked(on).on_select(
                    move |s: &mut App| s.model = Some(sel.with_option(&id, (!on).into())),
                ));
                continue;
            }
            menu = menu.group_label(d.label.clone());
            for o in &d.options {
                let (sel, id, pick) = (sel.clone(), d.id.clone(), o.id.clone());
                let mut item = MenuItem::new(o.label.clone()).checked(value == o.id.as_str());
                if let Some(desc) = &o.description {
                    item = item.shortcut(desc.clone());
                }
                menu = menu.item(item.on_select(move |s: &mut App| {
                    s.model = Some(sel.with_option(&id, pick.clone().into()))
                }));
            }
        }
        menu = menu.separator();
    }
    for p in s
        .config()
        .into_iter()
        .flat_map(|c| &c.providers)
        .filter(|p| p.usable() && !p.models.is_empty())
    {
        menu = menu.group_label(p.label().to_owned());
        for m in p
            .models
            .iter()
            .filter(|m| m.is_legacy != Some(true))
            .take(12)
        {
            let pick = ModelSelection {
                instance_id: p.instance_id.clone(),
                model: m.slug.clone(),
                options: None,
            };
            let on = current
                .as_ref()
                .is_some_and(|c| c.instance_id == pick.instance_id && c.model == pick.model);
            menu = menu.item(
                MenuItem::new(m.short_name.clone().unwrap_or_else(|| m.name.clone()))
                    .checked(on)
                    .on_select(move |s: &mut App| {
                        s.model = Some(pick.clone());
                        s.popup = None;
                    }),
            );
        }
    }
    menu.build(k, layer)
}

fn project_menu(s: &App, look: &Look, layer: &mut Vec<El>) -> El {
    let k = &look.kit;
    let label = s
        .draft_project
        .as_deref()
        .and_then(|p| s.shell().project(p))
        .map_or("Pick a project".to_owned(), |p| p.title.clone());
    let open = s.popup == Some(Popup::Project);
    let trigger = Button::new(label)
        .variant(ButtonVariant::Ghost)
        .control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.popup = if open { None } else { Some(Popup::Project) })
        .build(k)
        .key("project-trigger");
    let mut menu = Menu::new("project-menu")
        .trigger(trigger)
        .open(open)
        .min_width(220.0)
        .on_close(|s: &mut App| s.popup = None);
    // Every machine's projects; picking another machine's moves the draft there.
    let many = s.envs.len() > 1;
    for (env, e) in s.envs.iter().enumerate() {
        if many {
            menu = menu.group_label(s.env_name(env));
        }
        for p in e.shell.sidebar_projects() {
            let id = p.id.clone();
            menu = menu.item(
                MenuItem::new(p.title.clone())
                    .checked(env == s.focus && s.draft_project.as_deref() == Some(&p.id))
                    .on_select(move |s: &mut App| {
                        s.new_thread_in(env, id.clone());
                        s.popup = None;
                    }),
            );
        }
    }
    menu.build(k, layer)
}

// ---------------------------------------------------------------------------------------------

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `2026-11-06T12:00:00.000Z` → seconds since the epoch, without a date library.
fn parse_iso(s: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (
        n(0..4)?,
        n(5..7)?,
        n(8..10)?,
        n(11..13)?,
        n(14..16)?,
        n(17..19)?,
    );
    // Days from civil (Howard Hinnant).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146097 + doe - 719468) * 86400 + h * 3600 + mi * 60 + se)
}

fn age(iso: &str, now: i64) -> String {
    let Some(t) = parse_iso(iso) else {
        return String::new();
    };
    let d = (now - t).max(0);
    match d {
        0..60 => "now".into(),
        60..3600 => format!("{}m", d / 60),
        3600..86400 => format!("{}h", d / 3600),
        86400..604800 => format!("{}d", d / 86400),
        _ => format!("{}w", d / 604800),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_ages() {
        let t = super::parse_iso("2026-11-06T12:00:00.000Z").unwrap();
        assert_eq!(t, 1793966400);
        assert_eq!(super::age("2026-11-06T11:00:00Z", t), "1h");
    }
}
