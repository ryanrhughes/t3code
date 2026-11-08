//! Shortcut resolution over the server's resolved keybindings (the defaults plus the user's
//! `keybindings.json`, in `ServerConfig.keybindings`), ported from `resolveShortcutCommand`
//! (`apps/web/src/keybindings.ts`) and `packages/shared/src/keybindingMatching.ts`, and kept
//! equal to them by `tests/keybindings.rs`.
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub command: String,
    pub shortcut: Shortcut,
    #[serde(default)]
    pub when_ast: Option<When>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shortcut {
    pub key: String,
    pub meta_key: bool,
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub alt_key: bool,
    pub mod_key: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum When {
    Identifier { name: String },
    Not { node: Box<When> },
    And { left: Box<When>, right: Box<When> },
    Or { left: Box<When>, right: Box<When> },
}

/// A key press as the web sees it: `key` is the layout's name for it (`"k"`, `"{"`, `"Enter"`),
/// `code` the physical key (`"KeyK"`, `"BracketLeft"`).
#[derive(Clone, Debug, Default)]
pub struct KeyPress {
    pub key: String,
    pub code: Option<String>,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Decodes `ServerConfig.keybindings`, dropping rules this client can't read, as the web does.
pub fn decode(value: &serde_json::Value) -> Vec<Rule> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| serde_json::from_value(r.clone()).ok())
        .collect()
}

fn eval(node: &When, context: &dyn Fn(&str) -> bool) -> bool {
    match node {
        When::Identifier { name } if name == "true" => true,
        When::Identifier { name } if name == "false" => false,
        When::Identifier { name } => context(name),
        When::Not { node } => !eval(node, context),
        When::And { left, right } => eval(left, context) && eval(right, context),
        When::Or { left, right } => eval(left, context) || eval(right, context),
    }
}

/// `EVENT_CODE_SHORTCUT_KEYS`: punctuation and digits match by physical key.
fn code_key(code: &str) -> Option<&'static str> {
    Some(match code {
        "Backquote" => "`",
        "Backslash" => "\\",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Comma" => ",",
        "Digit0" => "0",
        "Digit1" => "1",
        "Digit2" => "2",
        "Digit3" => "3",
        "Digit4" => "4",
        "Digit5" => "5",
        "Digit6" => "6",
        "Digit7" => "7",
        "Digit8" => "8",
        "Digit9" => "9",
        "Equal" => "=",
        "Minus" => "-",
        "Period" => ".",
        "Quote" => "'",
        "Semicolon" => ";",
        "Slash" => "/",
        _ => return None,
    })
}

fn is_letter(key: &str) -> bool {
    key.len() == 1 && key.as_bytes()[0].is_ascii_lowercase()
}

/// `resolveEventKeys`: the names a press can match: the layout key, a letter's physical key
/// when the layout gives a non-letter, and punctuation by physical key.
fn event_keys(press: &KeyPress) -> Vec<String> {
    let mut layout = press.key.to_lowercase();
    if layout == "esc" {
        layout = "escape".into();
    }
    let mut keys = vec![layout.clone()];
    let code = press.code.as_deref().unwrap_or("");
    if let Some(letter) = code.strip_prefix("Key").filter(|l| l.len() == 1) {
        if !is_letter(&layout) {
            keys.push(letter.to_lowercase());
        }
    }
    let from_event = if is_letter(&layout) {
        layout
    } else {
        code_key(code).map_or(layout, str::to_owned)
    };
    keys.push(from_event);
    keys
}

fn matches(press: &KeyPress, s: &Shortcut, mac: bool) -> bool {
    let meta = s.meta_key || (s.mod_key && mac);
    let ctrl = s.ctrl_key || (s.mod_key && !mac);
    press.meta == meta
        && press.ctrl == ctrl
        && press.shift == s.shift_key
        && press.alt == s.alt_key
        && event_keys(press).contains(&s.key)
}

/// The command a press runs: the last rule whose `when` holds and whose shortcut matches.
pub fn resolve<'a>(
    press: &KeyPress,
    rules: &'a [Rule],
    context: &dyn Fn(&str) -> bool,
    mac: bool,
) -> Option<&'a str> {
    rules
        .iter()
        .rev()
        .filter(|r| r.when_ast.as_ref().is_none_or(|w| eval(w, context)))
        .find(|r| matches(press, &r.shortcut, mac))
        .map(|r| r.command.as_str())
}

/// The first shortcut that runs `command` with no context, as a label ("Ctrl Shift ["), for
/// buttons and the palette.
pub fn label(rules: &[Rule], command: &str, mac: bool) -> Option<String> {
    let s = &rules.iter().rev().find(|r| r.command == command)?.shortcut;
    let mut parts = Vec::new();
    if s.ctrl_key || (s.mod_key && !mac) {
        parts.push("Ctrl".to_owned());
    }
    if s.meta_key || (s.mod_key && mac) {
        parts.push("Cmd".to_owned());
    }
    if s.alt_key {
        parts.push("Alt".to_owned());
    }
    if s.shift_key {
        parts.push("Shift".to_owned());
    }
    let key = match s.key.as_str() {
        "arrowup" => "Up".to_owned(),
        "arrowdown" => "Down".to_owned(),
        "enter" => "Return".to_owned(),
        "escape" => "Esc".to_owned(),
        k if k.len() == 1 => k.to_uppercase(),
        k => {
            let mut c = k.chars();
            c.next()
                .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
        }
    };
    parts.push(key);
    Some(parts.join(" "))
}
