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
use crate::{App, Popup, Screen};

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
        (Some(_), Some(t)) => t.thread.title.clone(),
        (Some(id), None) => s
            .shell
            .thread(id)
            .map(|t| t.title.clone())
            .unwrap_or_default(),
        (None, _) => "New thread".to_owned(),
    };
    let toolbar = Toolbar::new()
        .title(title)
        .trailing(theme_menu(s, look, layer));
    let main = col()
        .key("main")
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(look.canvas)
        .child(timeline(s, look))
        .child(composer(s, look, layer));
    let working = s.shell.threads.iter().any(|t| t.is_working());
    SplitView::new(main)
        .sidebar(&s.sidebar, |s: &mut App| &mut s.sidebar, sidebar(s, look))
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
        id: String,
        title: String,
        count: usize,
        collapsed: bool,
    },
    Thread {
        id: String,
        title: String,
        status: Option<Status3>,
        age: String,
    },
}

#[derive(Clone, Copy)]
enum Status3 {
    Busy,
    Attention,
}

/// The sidebar as one flat list (project headers and their threads) so it virtualises.
fn entries(s: &App) -> Vec<Entry> {
    let now = now_secs();
    let mut out = Vec::new();
    for p in s.shell.sidebar_projects() {
        let threads = s.shell.project_threads(&p.id);
        let collapsed = s.collapsed.contains(&p.id);
        out.push(Entry::Project {
            id: p.id.clone(),
            title: p.title.clone(),
            count: threads.len(),
            collapsed,
        });
        if collapsed {
            continue;
        }
        for t in threads {
            let status = if t.needs_attention() {
                Some(Status3::Attention)
            } else if t.is_working() {
                Some(Status3::Busy)
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
            out.push(Entry::Thread {
                id: t.id.clone(),
                title: t.title.clone(),
                status,
                age: detail,
            });
        }
    }
    out
}

fn sidebar(s: &App, look: &Look) -> El {
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
    let (status_label, status_color) = match &s.status {
        Status::Connected => (
            s.cred
                .as_ref()
                .map_or("Connected".to_owned(), |c| c.label.clone()),
            look.muted,
        ),
        Status::Connecting => ("Connecting…".to_owned(), look.muted),
        Status::Reconnecting { reason, .. } => (format!("Reconnecting: {reason}"), look.warning_fg),
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
                .fill(if matches!(s.status, Status::Connected) {
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
        .child(
            Button::new("Unpair")
                .variant(ButtonVariant::GhostFaint)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.forget())
                .build(k)
                .key("unpair"),
        );
    let (key_rows, build_rows, kit) = (rows.clone(), rows.clone(), k.clone());
    let selected = s.selected.clone();
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
                Entry::Project { id, .. } => Cow::Owned(format!("p:{id}")),
                Entry::Thread { id, .. } => Cow::Owned(format!("t:{id}")),
            },
            move |_: &App, i| entry_row(&build_rows[i], selected.as_deref(), &kit),
        )
        .build(k)
}

fn entry_row(e: &Entry, selected: Option<&str>, k: &Kit) -> El {
    match e {
        Entry::Project {
            id,
            title,
            count,
            collapsed,
        } => {
            let (toggle_id, new_id) = (id.clone(), id.clone());
            SidebarHeader::new(title.clone())
                .count(*count)
                .collapsed(*collapsed)
                .on_toggle(move |s: &mut App, _| {
                    if !s.collapsed.remove(&toggle_id) {
                        s.collapsed.insert(toggle_id.clone());
                    }
                })
                .action(
                    IconButton::new(&PLUS)
                        .variant(IconButtonVariant::GhostMuted)
                        .label("New thread in project")
                        .on_press(move |s: &mut App| s.new_thread(Some(new_id.clone())))
                        .build(k)
                        .key("new-in-project"),
                )
                .build(k)
        }
        Entry::Thread {
            id,
            title,
            status,
            age,
        } => {
            let open = id.clone();
            let mut r = SidebarRow::new(title.clone())
                .height(32.0)
                .depth(1)
                .detail(age.clone())
                .selected(selected == Some(id.as_str()))
                .on_select(move |s: &mut App, _| s.open_thread(Some(open.clone())));
            r = match status {
                Some(Status3::Busy) => r.status(RowStatus::Other(
                    rune::icon(&WORKING, 14.0).color(k.c.text_accent),
                )),
                Some(Status3::Attention) => r.status(RowStatus::Attention),
                None => r,
            };
            r.build(k)
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
            let shown_rows: std::rc::Rc<Vec<usize>> = std::rc::Rc::new(
                (0..t.visible.len())
                    .filter(|&i| shown(&t.visible[i].item))
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
    let project = s.draft_project.as_deref().and_then(|p| s.shell.project(p));
    let hint = match project {
        Some(p) => format!(
            "Start a thread in {}. Messages run on {}.",
            p.title,
            s.cred.as_ref().map_or("the server", |c| c.label.as_str())
        ),
        None if !s.shell.loaded => "Loading projects…".to_owned(),
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
        "user_message" => row().w_full().justify_end().child(
            col()
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
                ),
        ),
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
            line
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
    let placeholder = if working {
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
        .on_submit(|s: &mut App, tx| s.send(tx));
    let field = col()
        .key("composer-field")
        .w_full()
        .px(4.0)
        .pt(2.0)
        .child(rune::auto_grow(editor, 2, 12));
    let send = if working && s.composer.text.trim().is_empty() {
        IconButton::new(&STOP)
            .variant(IconButtonVariant::Contrast)
            .label("Stop")
            .shortcut("Ctrl .")
            .on_press(|s: &mut App| s.stop())
            .build(k)
            .key("stop")
    } else {
        IconButton::new(&SEND)
            .variant(IconButtonVariant::Submit)
            .label("Send")
            .on_press_tx(|s: &mut App, tx| s.send(tx))
            .build(k)
            .key("send")
    };
    // Wraps to a second line rather than clipping in a narrow tiled window.
    let mut controls = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(model_menu(s, look, layer));
    if s.selected.is_none() {
        controls = controls.child(project_menu(s, look, layer));
    }
    if let Status::Reconnecting { .. } = &s.status {
        controls = controls.child(
            text("Offline — messages can't send")
                .font(k.ty.small)
                .color(look.warning_fg),
        );
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
        .child(controls.child(el().grow(1.0)).child(send));
    row()
        .key("composer-dock")
        .w_full()
        .justify_center()
        .px(24.0)
        .pt(4.0)
        .pb(16.0)
        .child(surface)
}

fn model_label(s: &App, m: &ModelSelection) -> String {
    let provider = s
        .config
        .as_ref()
        .and_then(|c| c.providers.iter().find(|p| p.instance_id == m.instance_id));
    let model = provider.and_then(|p| p.models.iter().find(|x| x.slug == m.model));
    model
        .map(|x| x.short_name.clone().unwrap_or_else(|| x.name.clone()))
        .unwrap_or_else(|| m.model.clone())
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
    for p in s
        .config
        .iter()
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
        .and_then(|p| s.shell.project(p))
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
    for p in s.shell.sidebar_projects() {
        let id = p.id.clone();
        menu = menu.item(
            MenuItem::new(p.title.clone())
                .checked(s.draft_project.as_deref() == Some(&p.id))
                .on_select(move |s: &mut App| {
                    s.draft_project = Some(id.clone());
                    s.popup = None;
                }),
        );
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
