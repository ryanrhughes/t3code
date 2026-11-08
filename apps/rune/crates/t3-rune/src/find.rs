//! Find in the open thread (the web's `chat.find`, Ctrl/Cmd+F): a bar over the timeline that
//! steps through the rows whose text matches, scrolling each into view.
use rune::{El, TextEdit, Tx, row, text};
use rune_kit::{IconButton, IconButtonVariant};

use crate::theme::Look;
use crate::{App, focused};

pub struct Find {
    pub query: TextEdit,
    /// Matching rows' keys (their source item ids), top to bottom.
    pub matches: Vec<String>,
    pub current: usize,
}

impl App {
    pub fn toggle_find(&mut self) {
        if self.find.is_some() {
            return self.close_find();
        }
        if self.thread.is_none() {
            return;
        }
        self.composer.focused = false;
        self.find = Some(Find {
            query: focused(""),
            matches: Vec::new(),
            current: 0,
        });
    }

    pub fn close_find(&mut self) {
        self.find = None;
        self.composer.focused = true;
    }

    /// Re-reads the matches after the query changes, and jumps to the first.
    pub fn refind(&mut self, tx: &mut Tx) {
        let (Some(f), Some(t)) = (self.find.as_ref(), self.thread.as_ref()) else {
            return;
        };
        let q = f.query.text.trim().to_lowercase();
        let matches: Vec<String> = if q.is_empty() {
            Vec::new()
        } else {
            t.visible
                .iter()
                .filter(|r| {
                    let i = &r.item;
                    [i.text.as_deref(), i.markdown.as_deref(), i.title.as_deref()]
                        .into_iter()
                        .flatten()
                        .any(|s| s.to_lowercase().contains(&q))
                })
                .map(|r| r.source_item_id.clone())
                .collect()
        };
        let f = self.find.as_mut().unwrap();
        f.matches = matches;
        f.current = 0;
        self.reveal_match(tx);
    }

    /// Steps to the next match (or the previous, with `back`), wrapping around.
    pub fn step_find(&mut self, back: bool, tx: &mut Tx) {
        let Some(f) = self.find.as_mut().filter(|f| !f.matches.is_empty()) else {
            return;
        };
        let n = f.matches.len();
        f.current = if back {
            (f.current + n - 1) % n
        } else {
            (f.current + 1) % n
        };
        self.reveal_match(tx);
    }

    fn reveal_match(&mut self, tx: &mut Tx) {
        if let Some(key) = self.find.as_ref().and_then(|f| f.matches.get(f.current)) {
            tx.scroll_into_view_in("rows", key.clone());
        }
    }

    pub fn find_key(&mut self, ev: &rune::KeyEvent, tx: &mut Tx) -> bool {
        match ev.key {
            rune::Key::Escape => self.close_find(),
            rune::Key::Enter => self.step_find(ev.mods.shift, tx),
            _ => return false,
        }
        true
    }
}

/// The find bar, over the top of the timeline.
pub fn bar(s: &App, look: &Look) -> Option<El> {
    let f = s.find.as_ref()?;
    let k = &look.kit;
    let field = rune::text_input(&f.query, k.ty.ui)
        .key("find-input")
        .w(220.0)
        .color(look.text)
        .placeholder("Find in thread", k.c.text_faint)
        .caret(look.text, k.selection())
        .label("Find in thread")
        .on_edit(|s: &mut App| &mut s.find.as_mut().expect("find open").query)
        .on_input_change(|s: &mut App, tx| s.refind(tx))
        .on_input_key(|s: &mut App, ev: &rune::KeyEvent, tx| s.find_key(ev, tx));
    let count = if f.query.text.trim().is_empty() {
        String::new()
    } else if f.matches.is_empty() {
        "No matches".into()
    } else {
        format!("{} of {}", f.current + 1, f.matches.len())
    };
    let step = |icon, label: &'static str, back: bool| {
        IconButton::new(icon)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(rune_kit::ControlSize::Small)
            .label(label)
            .on_press_tx(move |s: &mut App, tx| s.step_find(back, tx))
            .build(k)
    };
    Some(
        row()
            .key("find-bar")
            .gap(6.0)
            .px(10.0)
            .py(6.0)
            .center_y()
            .radius(k.radius.lg)
            .fill(look.raised)
            .border(1.0, look.border)
            .shadow(8.0, 24.0, rune::Color::BLACK.alpha(0.25))
            .child(field)
            .child(text(count).font(k.ty.small).color(look.muted))
            .child(step(&rune_kit::glyphs::CHEVRON_UP, "Previous match", true).key("find-prev"))
            .child(step(&rune_kit::glyphs::CHEVRON_DOWN, "Next match", false).key("find-next"))
            .child(
                IconButton::new(&rune_kit::glyphs::CLOSE)
                    .variant(IconButtonVariant::GhostMuted)
                    .control_size(rune_kit::ControlSize::Small)
                    .label("Close find")
                    .shortcut("Esc")
                    .on_press(|s: &mut App| s.close_find())
                    .build(k)
                    .key("find-close"),
            ),
    )
}
