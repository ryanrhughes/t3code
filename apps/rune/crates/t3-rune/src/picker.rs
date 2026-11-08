//! The file picker (the web's `filePicker.toggle`, Ctrl/Cmd+P): fuzzy-finds a file in the open
//! thread's workspace with `projects.searchEntries` and opens it in the Files tab.
use rune::{El, ScrollSpec, TextEdit, Tx, col, row, text};
use serde_json::json;

use crate::theme::Look;
use crate::{App, HANDLE, Inspector, focused};

pub struct FilePicker {
    pub query: TextEdit,
    pub active: usize,
    /// (path, is a folder), best first.
    pub results: Vec<(String, bool)>,
    pub loading: bool,
    seq: u64,
}

impl App {
    pub fn toggle_picker(&mut self) {
        if self.picker.is_some() {
            return self.close_picker();
        }
        if self.workspace_cwd().is_none() {
            return;
        }
        self.composer.focused = false;
        self.palette = None;
        self.picker = Some(FilePicker {
            query: focused(""),
            active: 0,
            results: Vec::new(),
            loading: false,
            seq: 0,
        });
        self.search_picker();
    }

    pub fn close_picker(&mut self) {
        self.picker = None;
        self.composer.focused = true;
    }

    /// Searches for the current query; only the newest answer fills the list.
    pub fn search_picker(&mut self) {
        let (Some(cwd), Some(session), Some(handle)) =
            (self.workspace_cwd(), self.session(), HANDLE.get().cloned())
        else {
            return;
        };
        let Some(p) = self.picker.as_mut() else {
            return;
        };
        p.seq += 1;
        p.loading = true;
        p.active = 0;
        let seq = p.seq;
        let input = json!({ "cwd": cwd, "query": p.query.text.trim(), "limit": 50 });
        session.call("projects.searchEntries", input, move |result| {
            handle.update(move |s: &mut App, _| {
                let Some(p) = s.picker.as_mut().filter(|p| p.seq == seq) else {
                    return;
                };
                p.loading = false;
                p.results = result
                    .ok()
                    .and_then(|v| v["entries"].as_array().cloned())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|e| {
                        Some((e["path"].as_str()?.to_owned(), e["kind"] == "directory"))
                    })
                    // Files first: a folder can't open in the source view.
                    .filter(|(_, dir)| !dir)
                    .collect();
            });
        });
    }

    /// Opens a picked file in the Files tab.
    pub fn pick_file(&mut self, i: usize) {
        let Some(path) = self
            .picker
            .as_ref()
            .and_then(|p| p.results.get(i))
            .map(|(path, _)| path.clone())
        else {
            return;
        };
        self.close_picker();
        self.show_inspector(Inspector::Files);
        self.open_file(&path);
    }

    pub fn picker_key(&mut self, key: &rune::Key, _tx: &mut Tx) -> bool {
        let Some(p) = self.picker.as_mut() else {
            return false;
        };
        let n = p.results.len();
        match key {
            rune::Key::Escape => self.close_picker(),
            rune::Key::Down if n > 0 => p.active = (p.active + 1) % n,
            rune::Key::Up if n > 0 => p.active = (p.active + n - 1) % n,
            rune::Key::Enter => {
                let i = p.active;
                self.pick_file(i);
            }
            _ => return false,
        }
        true
    }
}

pub fn overlay(s: &App, look: &Look) -> Option<El> {
    let p = s.picker.as_ref()?;
    let k = &look.kit;
    let field = rune::text_input(&p.query, k.ty.body)
        .key("picker-input")
        .w_full()
        .color(look.text)
        .placeholder("Find a file", k.c.text_faint)
        .caret(look.text, k.selection())
        .label("Find a file")
        .on_edit(|s: &mut App| &mut s.picker.as_mut().expect("picker open").query)
        .on_input_change(|s: &mut App, _| s.search_picker())
        .on_input_key(|s: &mut App, ev: &rune::KeyEvent, tx| s.picker_key(&ev.key, tx));
    let mut list = col().key("picker-rows").w_full().gap(2.0);
    for (i, (path, _)) in p.results.iter().enumerate() {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        let mut r = row()
            .key(format!("picker-{i}"))
            .w_full()
            .px(10.0)
            .py(6.0)
            .gap(10.0)
            .center_y()
            .radius(k.radius.md)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |s: &mut App| s.pick_file(i))
            .child(rune::icon(rune_kit::file_icon(name), 14.0).color(look.muted))
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
                    .grow(1.0)
                    .shrink(),
            );
        if i == p.active {
            r = r.fill(k.c.bg_accent.alpha(0.35));
        }
        list = list.child(r);
    }
    if p.results.is_empty() {
        list = list.child(
            text(if p.loading {
                "Searching…"
            } else {
                "No matching files"
            })
            .font(k.ty.ui)
            .color(look.muted)
            .padding(12.0),
        );
    }
    let card = col()
        .key("picker-card")
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
    Some(
        col()
            .key("picker-overlay")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(rune::Color::BLACK.alpha(0.25))
            .items_center()
            .pt(80.0)
            .px(16.0)
            .on_press(|s: &mut App| s.close_picker())
            .child(card.on_press(|_: &mut App| {})),
    )
}
