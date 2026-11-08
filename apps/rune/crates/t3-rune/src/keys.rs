//! Keyboard shortcuts from the server's resolved keybindings (the web's defaults plus the
//! user's keybindings.json), so a binding changed on the web works here too. Commands this
//! client doesn't have pass through.
use rune::{Key, KeyEvent, Tx};
use t3_client::keybindings::{KeyPress, resolve};

use crate::{App, Popup};

/// The web's name for a key (`KeyboardEvent.key`), and its physical key (`.code`).
fn press(ev: &KeyEvent, tx: &Tx) -> Option<KeyPress> {
    let key = match &ev.key {
        Key::Text(t) => t.clone(),
        Key::Command(c) => c.to_string(),
        Key::Enter => "Enter".into(),
        Key::Escape => "Escape".into(),
        Key::Tab => "Tab".into(),
        Key::Backspace => "Backspace".into(),
        Key::Delete => "Delete".into(),
        Key::Up => "ArrowUp".into(),
        Key::Down => "ArrowDown".into(),
        Key::Left => "ArrowLeft".into(),
        Key::Right => "ArrowRight".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PageUp".into(),
        Key::PageDown => "PageDown".into(),
        Key::Function(n) => format!("F{n}"),
        _ => return None,
    };
    let mac = cfg!(target_os = "macos");
    Some(KeyPress {
        key,
        // winit's key codes carry the web's `code` names.
        code: tx.physical_key().map(|c| format!("{c:?}")),
        ctrl: ev.mods.ctrl || (ev.mods.cmd && !mac),
        meta: ev.mods.cmd && mac,
        shift: ev.mods.shift,
        alt: ev.mods.alt,
    })
}

impl App {
    /// The `when` context the web evaluates bindings in.
    fn key_context(&self, name: &str) -> bool {
        let overlay = self.palette.is_some() || self.rename.is_some() || self.picker.is_some();
        match name {
            "composerFocus" => self.composer.focused && !overlay,
            "editableFocus" => self.composer.focused || overlay,
            "rightPanelOpen" => self.diff_pane.is_open(),
            "modelPickerOpen" => self.popup == Some(Popup::Model),
            "turnRunning" => self.thread.as_ref().is_some_and(|t| t.is_working()),
            "draftThreadRoute" => self.selected.is_none(),
            "isDesktop" => true,
            _ => false,
        }
    }

    /// Runs the command a press is bound to; false when it's unbound or not one of ours.
    pub fn run_shortcut(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let Some(press) = press(ev, tx) else {
            return false;
        };
        let Some(rules) = self.env().map(|e| e.keys.clone()) else {
            return false;
        };
        let Some(command) = resolve(
            &press,
            &rules,
            &|n| self.key_context(n),
            cfg!(target_os = "macos"),
        ) else {
            return false;
        };
        self.run_command(command, tx)
    }

    pub fn run_command(&mut self, command: &str, tx: &mut Tx) -> bool {
        match command {
            "commandPalette.toggle" => self.toggle_palette(),
            "filePicker.toggle" => self.toggle_picker(),
            "sidebar.toggle" => self.sidebar.toggle(),
            "diff.toggle" => self.toggle_diff(),
            "rightPanel.toggle" => self.toggle_inspector(self.inspector),
            "rightPanel.close" if self.diff_pane.is_open() => self.diff_pane.set_open(false),
            "chat.new" | "chat.newLocal" => self.new_thread(None),
            "chat.find" => self.toggle_find(),
            "thread.previous" => self.step_thread(-1),
            "thread.next" => self.step_thread(1),
            "modelPicker.toggle" => {
                self.popup = (self.popup != Some(Popup::Model)).then_some(Popup::Model)
            }
            "composer.mode" => self.toggle_plan_mode(),
            "thread.steerQueuedMessage" => return self.send_next_queued(),
            "thread.editQueuedMessage" => return self.edit_last_queued(),
            "composer.sendAlternate" if self.composer.focused => self.send(true, tx),
            _ => return false,
        }
        true
    }

    /// The thread above or below the open one, in sidebar order, wrapping around.
    fn step_thread(&mut self, by: isize) {
        let order = crate::view::thread_order(self);
        if order.is_empty() {
            return;
        }
        let here = order.iter().position(|(env, id)| {
            *env == self.focus && self.selected.as_deref() == Some(id.as_str())
        });
        let next = match here {
            Some(i) => (i as isize + by).rem_euclid(order.len() as isize) as usize,
            None if by > 0 => 0,
            None => order.len() - 1,
        };
        let (env, id) = order[next].clone();
        self.open_thread_in(env, id);
    }
}
