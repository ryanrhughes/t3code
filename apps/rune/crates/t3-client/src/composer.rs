//! Composer triggers and inserted tokens, ported from `packages/shared/src/composerTrigger.ts`
//! and kept equal to it by `tests/composer.rs`. Offsets are UTF-8 byte offsets, as `rune::TextEdit`
//! keeps its caret; the TypeScript's are UTF-16, which only matters for where non-ASCII text sits.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    /// `@query`: files and folders in the workspace.
    Path,
    /// `#123`: a pull request.
    PullRequest,
    /// `/query`: client and provider commands.
    SlashCommand,
    /// `/model query`: the model picker.
    SlashModel,
    /// `$query`: provider skills.
    Skill,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub kind: TriggerKind,
    pub query: String,
    /// The bytes a pick replaces.
    pub start: usize,
    pub end: usize,
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\n' | '\t' | '\r')
}

/// A second slash makes the whole word a path, even when the caret precedes that slash.
fn is_path_word(text: &str, word_start: usize, cursor: usize) -> bool {
    let rest = &text[cursor..];
    let word_end = cursor + rest.find(is_ws).unwrap_or(rest.len());
    text[word_start + 1..word_end].contains('/')
}

/// `$5`, `€9.99` and `$10k` are money, not skills.
fn is_currency_amount(query: &str) -> bool {
    let q = query.strip_prefix('.').unwrap_or(query);
    let mut chars = q.chars().peekable();
    if !chars.next().is_some_and(|c| c.is_ascii_digit()) {
        return false;
    }
    while chars
        .peek()
        .is_some_and(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | '_'))
    {
        chars.next();
    }
    match chars.peek() {
        Some('k' | 'K' | 'm' | 'M' | 'b' | 'B' | 't' | 'T') => {
            chars.next();
        }
        Some('e' | 'E') => {
            let mut look = chars.clone();
            look.next();
            if look.peek().is_some_and(|c| c.is_ascii_digit()) {
                chars.next();
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    chars.next();
                }
            }
        }
        _ => {}
    }
    chars.all(|c| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')'))
}

/// Unicode currency symbols (`\p{Sc}`), the skill prefix.
fn is_currency_symbol(c: char) -> bool {
    matches!(
        c,
        '$' | '¢'
            | '£'
            | '¤'
            | '¥'
            | '֏'
            | '؋'
            | '߾'
            | '߿'
            | '৲'
            | '৳'
            | '৻'
            | '૱'
            | '௹'
            | '฿'
            | '៛'
    ) || ('\u{20A0}'..='\u{20C0}').contains(&c)
        || matches!(c, '꠸' | '﷼' | '﹩' | '＄' | '￠' | '￡' | '￥' | '￦')
}

/// `detectComposerTrigger`: the token the caret is in, if it opens a menu.
pub fn detect(text: &str, cursor: usize) -> Option<Trigger> {
    let mut cursor = cursor.min(text.len());
    while !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let line_start = text[..cursor].rfind('\n').map_or(0, |i| i + 1);
    let line_prefix = &text[line_start..cursor];
    if let Some(command) = line_prefix.strip_prefix('/') {
        if !command.contains(char::is_whitespace) {
            if is_path_word(text, line_start, cursor) {
                return None;
            }
            if command.eq_ignore_ascii_case("model") {
                return Some(Trigger {
                    kind: TriggerKind::SlashModel,
                    query: String::new(),
                    start: line_start,
                    end: cursor,
                });
            }
            return Some(Trigger {
                kind: TriggerKind::SlashCommand,
                query: command.into(),
                start: line_start,
                end: cursor,
            });
        }
        // `/model <query>`: the rest of the line filters the model picker.
        if line_prefix.len() >= 6 && line_prefix[..6].eq_ignore_ascii_case("/model") {
            let rest = &line_prefix[6..];
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return Some(Trigger {
                    kind: TriggerKind::SlashModel,
                    query: rest.trim().into(),
                    start: line_start,
                    end: cursor,
                });
            }
        }
    }
    let token_start = text[..cursor].rfind(is_ws).map_or(0, |i| i + 1);
    let token = &text[token_start..cursor];
    if let Some(query) = token.strip_prefix('/') {
        if is_path_word(text, token_start, cursor) {
            return None;
        }
        return Some(Trigger {
            kind: TriggerKind::SlashCommand,
            query: query.into(),
            start: token_start,
            end: cursor,
        });
    }
    if let Some(query) = token.strip_prefix('#') {
        let mut chars = query.chars();
        let ok = match chars.next() {
            None => true,
            Some(first) => {
                first.is_alphanumeric()
                    && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            }
        };
        if ok {
            return Some(Trigger {
                kind: TriggerKind::PullRequest,
                query: query.into(),
                start: token_start,
                end: cursor,
            });
        }
    }
    if let Some(first) = token.chars().next().filter(|c| is_currency_symbol(*c)) {
        let query = &token[first.len_utf8()..];
        if is_currency_amount(query) {
            return None;
        }
        return Some(Trigger {
            kind: TriggerKind::Skill,
            query: query.into(),
            start: token_start,
            end: cursor,
        });
    }
    token.strip_prefix('@').map(|query| Trigger {
        kind: TriggerKind::Path,
        query: query.into(),
        start: token_start,
        end: cursor,
    })
}

/// `serializeComposerFileLink`: how a picked file travels in the message, `[name](path)`.
pub fn file_link(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let label = base
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    format!("[{label}]({})", encode_destination(path))
}

/// `encodeURI`, then `( ) # ? \` escaped too, as the TypeScript does for link destinations.
fn encode_destination(path: &str) -> String {
    const KEEP: &str = ";,/?:@&=+$-_.!~*'()#";
    let mut out = String::new();
    for c in path.chars() {
        let escape = match c {
            '(' | ')' | '#' | '?' | '\\' => true,
            c if c.is_ascii_alphanumeric() || KEEP.contains(c) => false,
            _ => true,
        };
        if escape {
            let mut buf = [0; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caret_in_a_mention_opens_the_file_menu() {
        let t = detect("look at @src/ma", 15).unwrap();
        assert_eq!(
            (t.kind, t.query.as_str(), t.start, t.end),
            (TriggerKind::Path, "src/ma", 8, 15)
        );
        assert_eq!(detect("costs $5 now", 8), None);
        assert_eq!(
            file_link("src/a b(1).rs"),
            "[a b(1).rs](src/a%20b%281%29.rs)"
        );
    }
}
