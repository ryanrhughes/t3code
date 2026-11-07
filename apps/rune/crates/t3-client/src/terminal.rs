//! Command output as the web shows it: a port of `packages/shared/src/terminalOutput.ts`, kept
//! byte-for-byte equal to it by `tests/terminal.rs` (fixtures from the real TypeScript).
//!
//! Only the last `LIVE_LINES` lines are cursor-addressable and nothing wraps; older lines settle
//! into plain text with SGR resets at line boundaries, so the tail can be cut anywhere safely.
//! Lengths count `char`s where the TypeScript counts UTF-16 units: they differ only for
//! characters outside the Basic Multilingual Plane (emoji), which only shifts where a cut lands.

/// Default command tail, as `TERMINAL_OUTPUT_TAIL_CHARS`.
pub const TAIL_CHARS: usize = 64 * 1024;
const LIVE_LINES: usize = 64;
const MAX_PENDING_ESCAPE: usize = 64;
const MAX_LINE_RUNS: usize = 2_048;
const ESC: char = '\u{1b}';

/// One character and its canonical SGR parameters ("" is the default style).
#[derive(Clone, Debug, PartialEq)]
struct Cell {
    ch: char,
    pen: std::sync::Arc<str>,
}

type Line = Vec<Cell>;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pos {
    row: usize,
    col: usize,
}

/// One command's output as a client holds it.
#[derive(Clone, Debug)]
pub struct TerminalOutput {
    /// Settled lines then the live window; contains only SGR escapes.
    pub text: String,
    pub truncated: bool,
    /// An unfinished escape sequence awaiting the next chunk.
    pending: Vec<char>,
    settled: String,
    lines: Vec<Line>,
    row: usize,
    col: usize,
    pen: std::sync::Arc<str>,
    saved: Option<Pos>,
    max_chars: usize,
}

impl Default for TerminalOutput {
    fn default() -> Self {
        Self::with_max(TAIL_CHARS)
    }
}

impl TerminalOutput {
    pub fn with_max(max_chars: usize) -> Self {
        TerminalOutput {
            text: String::new(),
            truncated: false,
            pending: Vec::new(),
            settled: String::new(),
            lines: vec![Vec::new()],
            row: 0,
            col: 0,
            pen: "".into(),
            saved: None,
            max_chars,
        }
    }

    /// Unfinished escape text held for the next chunk.
    pub fn pending(&self) -> String {
        self.pending.iter().collect()
    }

    /// `appendTerminalOutput`.
    pub fn append(&mut self, chunk: &str) {
        let mut input = std::mem::take(&mut self.pending);
        input.extend(chunk.chars());
        if input.is_empty() {
            return;
        }
        let max = self.max_chars;
        let mut t = Fold {
            out: self,
            dropped: false,
        };
        t.out.pending = t.feed(&input);
        let mut window: usize = t.out.lines.iter().map(Vec::len).sum();
        while window > max && t.out.row > 0 {
            window -= t.scroll_out();
        }
        let dropped = t.dropped;
        // Lines below a cursor that moved up can't scroll out; they keep only their ends.
        if window > max {
            let mut budget = max as i64
                - self.lines[..=self.row]
                    .iter()
                    .map(|l| l.len() as i64)
                    .sum::<i64>();
            for row in self.row + 1..self.lines.len() {
                let len = self.lines[row].len();
                let keep = budget.clamp(0, len as i64) as usize;
                if keep < len {
                    self.truncated = true;
                    self.lines[row].drain(..len - keep);
                    if let Some(s) = self.saved.as_mut().filter(|s| s.row == row) {
                        s.col = s.col.saturating_sub(len - keep);
                    }
                }
                budget -= keep as i64;
            }
        }
        if dropped {
            self.truncated = true;
        }
        let (settled, settled_cut) = tail(&self.settled, max);
        self.settled = settled;
        let window = self
            .lines
            .iter()
            .map(|l| serialize_line(l))
            .collect::<Vec<_>>()
            .join("\n");
        let (text, text_cut) = tail(&format!("{}{window}", self.settled), max);
        self.text = text;
        self.truncated |= settled_cut || text_cut;
    }

    /// Applies one `subscribeCommandOutput` frame: a `replace` carries the whole bounded tail,
    /// an `append` raw terminal text (`applyCommandOutputFrame`).
    pub fn apply_frame(&mut self, replace: bool, text: &str, truncated: bool) {
        if replace {
            *self = Self::with_max(self.max_chars);
        }
        self.truncated |= truncated;
        self.append(text);
    }
}

/// The `Terminal` of one feed: cursor moves and writes against the output's screen.
struct Fold<'a> {
    out: &'a mut TerminalOutput,
    /// Output was discarded on purpose (scrollback cleared or a line over budget).
    dropped: bool,
}

impl Fold<'_> {
    fn write(&mut self, text: &[char]) {
        let o = &mut *self.out;
        let pen = o.pen.clone();
        let line = &mut o.lines[o.row];
        if o.col > line.len() {
            line.resize(
                o.col,
                Cell {
                    ch: ' ',
                    pen: "".into(),
                },
            );
        }
        for (i, ch) in text.iter().enumerate() {
            let cell = Cell {
                ch: *ch,
                pen: pen.clone(),
            };
            match line.get_mut(o.col + i) {
                Some(c) => *c = cell,
                None => line.push(cell),
            }
        }
        o.col += text.len();
        // Flatten the oldest styles so color-per-character lines stay cheap.
        let runs = run_starts(line);
        if runs.len() > MAX_LINE_RUNS {
            let keep = runs.len() - MAX_LINE_RUNS / 8;
            let plain_end = runs[keep];
            for c in &mut line[..plain_end] {
                c.pen = "".into();
            }
        }
        if line.len() > o.max_chars {
            let removed = line.len() - o.max_chars;
            line.drain(..removed);
            self.dropped = true;
            o.col = o.col.saturating_sub(removed);
            if let Some(s) = o.saved.as_mut().filter(|s| s.row == o.row) {
                s.col = s.col.saturating_sub(removed);
            }
        }
    }

    fn newline(&mut self) {
        let o = &mut *self.out;
        o.row += 1;
        o.col = 0;
        if o.row == o.lines.len() {
            o.lines.push(Vec::new());
        }
        while self.out.lines.len() > LIVE_LINES {
            self.scroll_out();
        }
    }

    fn scroll_out(&mut self) -> usize {
        let o = &mut *self.out;
        let line = o.lines.remove(0);
        o.settled.push_str(&serialize_line(&line));
        o.settled.push('\n');
        // Settled text is re-cut only past twice the budget, as the TypeScript's head/tail parts.
        if o.settled.chars().count() > 2 * o.max_chars {
            o.settled = tail(&o.settled, o.max_chars).0;
            self.dropped = true;
        }
        o.row = o.row.saturating_sub(1);
        if let Some(s) = o.saved.as_mut() {
            s.row = s.row.saturating_sub(1);
        }
        line.len()
    }

    fn move_to(&mut self, row: i64, col: i64) {
        let o = &mut *self.out;
        o.row = row.clamp(0, o.lines.len() as i64 - 1) as usize;
        // A huge column (ESC[999999999C) would otherwise pad a line past any limit.
        o.col = col.clamp(0, TAIL_CHARS as i64) as usize;
    }

    fn csi(&mut self, params: &str, fin: char) {
        if !params
            .chars()
            .all(|c| c.is_ascii_digit() || c == ';' || c == ':')
            || (fin != 'm' && params.contains(':'))
        {
            return;
        }
        if fin == 'm' {
            let mut attrs = parse_pen(&self.out.pen);
            apply_sgr(
                &mut attrs,
                &if params.is_empty() {
                    vec![]
                } else {
                    csi_params(params)
                },
            );
            self.out.pen = format_pen(&attrs).into();
            return;
        }
        let numbers = if params.is_empty() {
            vec![]
        } else {
            csi_params(params)
        };
        let first = numbers.first().copied();
        let count = first.filter(|n| *n != 0).unwrap_or(1).max(1);
        let (row, col) = (self.out.row as i64, self.out.col as i64);
        match fin {
            'A' => self.move_to(row - count, col),
            'B' => self.move_to(row + count, col),
            'C' => self.move_to(row, col + count),
            'D' => self.move_to(row, col - count),
            'E' => self.move_to(row + count, 0),
            'F' => self.move_to(row - count, 0),
            'G' => self.move_to(row, count - 1),
            // Rows count from the top of the window: there is no fixed screen height.
            'H' | 'f' => self.move_to(
                count - 1,
                numbers
                    .get(1)
                    .copied()
                    .filter(|n| *n != 0)
                    .unwrap_or(1)
                    .max(1)
                    - 1,
            ),
            'K' => {
                let o = &mut *self.out;
                o.lines[o.row] = erase_line(&o.lines[o.row], o.col, first.unwrap_or(0));
            }
            'J' => {
                let o = &mut *self.out;
                match first.unwrap_or(0) {
                    0 => {
                        o.lines[o.row] = erase_line(&o.lines[o.row], o.col, 0);
                        o.lines.truncate(o.row + 1);
                    }
                    1 => {
                        for r in 0..o.row {
                            o.lines[r].clear();
                        }
                        o.lines[o.row] = erase_line(&o.lines[o.row], o.col, 1);
                    }
                    3 => {
                        if !o.settled.is_empty() {
                            self.dropped = true;
                        }
                        self.out.settled.clear();
                    }
                    2 => {
                        o.lines = vec![Vec::new()];
                        o.row = 0;
                        o.col = 0;
                    }
                    _ => {}
                }
            }
            's' => {
                self.out.saved = Some(Pos {
                    row: self.out.row,
                    col: self.out.col,
                })
            }
            'u' => {
                if let Some(s) = self.out.saved {
                    self.move_to(s.row as i64, s.col as i64);
                }
            }
            _ => {}
        }
    }

    fn escape(&mut self, seq: &[char]) {
        match seq {
            [_, '[', .., fin] => {
                let params: String = seq[2..seq.len() - 1].iter().collect();
                self.csi(&params, *fin);
            }
            [_, '7'] => {
                self.out.saved = Some(Pos {
                    row: self.out.row,
                    col: self.out.col,
                })
            }
            [_, '8'] => {
                if let Some(s) = self.out.saved {
                    self.move_to(s.row as i64, s.col as i64);
                }
            }
            [_, 'M'] => self.move_to(self.out.row as i64 - 1, self.out.col as i64),
            _ => {}
        }
    }

    /// Applies `input`; returns the unfinished escape left at its end.
    fn feed(&mut self, input: &[char]) -> Vec<char> {
        let mut printable = 0;
        let mut i = 0;
        while i < input.len() {
            let code = input[i] as u32;
            if code == 0x09 || (code >= 0x20 && code != 0x7f && !(0x80..=0x9f).contains(&code)) {
                i += 1;
                continue;
            }
            if i > printable {
                self.write(&input[printable..i]);
            }
            printable = i + 1;
            match code {
                0x0a => self.newline(),
                0x0d => self.out.col = 0,
                0x08 => self.out.col = self.out.col.saturating_sub(1),
                0x1b => match escape_length(input, i) {
                    None => {
                        let fragment = &input[i..];
                        if fragment.len() <= MAX_PENDING_ESCAPE {
                            return fragment.to_vec();
                        }
                        // Keep the introducer (and a possible ST start) so a long string payload stays hidden.
                        if STRING_INTRODUCERS.contains(&fragment[1]) {
                            let mut kept = fragment[..2].to_vec();
                            if fragment.last() == Some(&ESC) {
                                kept.push(ESC);
                            }
                            return kept;
                        }
                        return Vec::new();
                    }
                    Some(len) => {
                        self.escape(&input[i..i + len]);
                        i += len - 1;
                        printable = i + 1;
                    }
                },
                _ => {}
            }
            i += 1;
        }
        if input.len() > printable {
            self.write(&input[printable..]);
        }
        Vec::new()
    }
}

const STRING_INTRODUCERS: [char; 5] = [']', 'P', 'X', '_', '^'];

/// Length of a complete escape sequence starting at `i` (an ESC), or `None` if unfinished.
fn escape_length(input: &[char], i: usize) -> Option<usize> {
    let next = *input.get(i + 1)?;
    // A second ESC restarts the sequence.
    if next == ESC {
        return Some(1);
    }
    if next == '[' {
        return (i + 2..input.len())
            .find(|&c| (0x40..=0x7e).contains(&(input[c] as u32)))
            .map(|c| c - i + 1);
    }
    if STRING_INTRODUCERS.contains(&next) {
        // BEL/ST ends string sequences; a newline also stops them so stray introducers can't hide output.
        for c in i + 2..input.len() {
            match input[c] {
                '\u{7}' => return Some(c - i + 1),
                '\n' => return Some(c - i),
                // An ESC that doesn't start ST ends the string and begins the next sequence (xterm.js).
                ESC => {
                    let after = *input.get(c + 1)?;
                    return Some(if after == '\\' { c - i + 2 } else { c - i });
                }
                _ => {}
            }
        }
        return None;
    }
    // ECMA-35: intermediate bytes (0x20-0x2F) precede one final byte.
    let mut c = i + 1;
    while c < input.len() && (0x20..=0x2f).contains(&(input[c] as u32)) {
        c += 1;
    }
    if c >= input.len() {
        return None;
    }
    let fin = input[c] as u32;
    // After intermediates, a control or ESC isn't a final byte; end before it.
    if c > i + 1 && !(0x30..=0x7e).contains(&fin) {
        return Some(c - i);
    }
    Some(c - i + 1)
}

/// Start index of every run of equal pens in `line`.
fn run_starts(line: &Line) -> Vec<usize> {
    let mut starts = Vec::new();
    for (i, c) in line.iter().enumerate() {
        if i == 0 || line[i - 1].pen != c.pen {
            starts.push(i);
        }
    }
    starts
}

/// Erase in line: 0 cursor to end, 1 start to cursor, 2 whole line.
fn erase_line(line: &Line, col: usize, mode: i64) -> Line {
    let len = line.len();
    match mode {
        0 => line[..col.min(len)].to_vec(),
        1 => {
            let blank = (col + 1).min(len);
            let mut out: Line = (0..blank)
                .map(|_| Cell {
                    ch: ' ',
                    pen: "".into(),
                })
                .collect();
            out.extend_from_slice(&line[(col + 1).min(len)..]);
            out
        }
        2 => Vec::new(),
        _ => line.clone(),
    }
}

fn sgr(pen: &str) -> String {
    if pen.is_empty() {
        format!("{ESC}[0m")
    } else {
        format!("{ESC}[0;{pen}m")
    }
}

fn serialize_line(line: &Line) -> String {
    let mut out = String::new();
    let mut current: &str = "";
    for c in line {
        if &*c.pen != current {
            out.push_str(&sgr(&c.pen));
            current = &c.pen;
        }
        out.push(c.ch);
    }
    if !current.is_empty() {
        out.push_str(&sgr(""));
    }
    out
}

#[derive(Clone, Default)]
struct Attrs {
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    inverse: bool,
    fg: String,
    bg: String,
}

fn parse_pen(pen: &str) -> Attrs {
    let mut a = Attrs::default();
    if !pen.is_empty() {
        apply_sgr(&mut a, &pen.split(';').map(js_number).collect::<Vec<_>>());
    }
    a
}

/// `Number(part)` on a part already checked to be digits or empty.
fn js_number(part: &str) -> i64 {
    part.parse().unwrap_or(0)
}

fn format_pen(a: &Attrs) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (on, code) in [
        (a.bold, "1"),
        (a.dim, "2"),
        (a.italic, "3"),
        (a.underline, "4"),
        (a.inverse, "7"),
        (a.strike, "9"),
    ] {
        if on {
            parts.push(code.into());
        }
    }
    if !a.fg.is_empty() {
        parts.push(a.fg.clone());
    }
    if !a.bg.is_empty() {
        parts.push(a.bg.clone());
    }
    parts.join(";")
}

/// Reads `5;n` or `2;r;g;b` colours; returns the code and the next index.
fn extended_color(code: i64, params: &[i64], i: usize) -> (Option<String>, usize) {
    match params.get(i + 1) {
        Some(5) => match params.get(i + 2) {
            Some(v) if *v <= 255 => (Some(format!("{code};5;{v}")), i + 3),
            _ => (None, i + 3),
        },
        Some(2) => match (params.get(i + 2), params.get(i + 3), params.get(i + 4)) {
            (Some(r), Some(g), Some(b)) => (
                Some(format!("{code};2;{};{};{}", r & 255, g & 255, b & 255)),
                i + 5,
            ),
            _ => (None, i + 5),
        },
        _ => (None, i + 2),
    }
}

fn apply_sgr(a: &mut Attrs, params: &[i64]) {
    let reset = [0];
    let params = if params.is_empty() {
        &reset[..]
    } else {
        params
    };
    let mut i = 0;
    while i < params.len() {
        let code = params[i];
        match code {
            0 => *a = Attrs::default(),
            1 => a.bold = true,
            2 => a.dim = true,
            3 => a.italic = true,
            4 => a.underline = true,
            7 => a.inverse = true,
            9 => a.strike = true,
            22 => (a.bold, a.dim) = (false, false),
            23 => a.italic = false,
            24 => a.underline = false,
            27 => a.inverse = false,
            29 => a.strike = false,
            30..=37 | 90..=97 => a.fg = code.to_string(),
            40..=47 | 100..=107 => a.bg = code.to_string(),
            39 => a.fg.clear(),
            49 => a.bg.clear(),
            38 | 48 | 58 => {
                // 58 (underline colour) is consumed but not kept: nothing renders it.
                let (color, next) = extended_color(code, params, i);
                if let Some(c) = color.filter(|_| code != 58) {
                    if code == 38 { a.fg = c } else { a.bg = c }
                }
                i = next;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
}

/// SGR parameters with colon sub-parameters folded the way the TypeScript does.
fn csi_params(params: &str) -> Vec<i64> {
    enum Pending {
        Code,
        Mode,
        Values(i64),
    }
    let mut numbers = Vec::new();
    let mut pending = Pending::Code;
    for part in params.split(';') {
        let mut sub: Vec<i64> = part.split(':').map(js_number).collect();
        let head = sub[0];
        match pending {
            Pending::Values(n) => {
                numbers.push(head);
                pending = if n > 1 {
                    Pending::Values(n - 1)
                } else {
                    Pending::Code
                };
                continue;
            }
            Pending::Mode => {
                pending = Pending::Code;
                if sub.len() == 1 {
                    numbers.push(head);
                    if head == 5 || head == 2 {
                        pending = Pending::Values(if head == 5 { 1 } else { 3 });
                    }
                    continue;
                }
                // xterm also takes the mode's fields after colons (`38;2:255:0:0`).
                if head == 2 && sub.len() >= 5 {
                    sub.remove(1);
                }
                let n = if head == 5 {
                    2
                } else if head == 2 {
                    4
                } else {
                    1
                };
                numbers.extend(sub.iter().take(n));
                continue;
            }
            Pending::Code => {}
        }
        if sub.len() == 1 {
            numbers.push(head);
            if head == 38 || head == 48 || head == 58 {
                pending = Pending::Mode;
            }
            continue;
        }
        if head != 38 && head != 48 && head != 58 {
            // Other sub-parameters pick a style variant; keep the head, but `4:0` is underline off.
            numbers.push(if head == 4 && sub[1] == 0 { 24 } else { head });
            continue;
        }
        if sub[1] == 2 && sub.len() >= 6 {
            sub.remove(2);
        }
        let n = if sub[1] == 5 {
            3
        } else if sub[1] == 2 {
            5
        } else {
            2
        };
        numbers.extend(sub.iter().take(n));
    }
    numbers
}

/// Keeps the last `max` chars, preferring to start on a fresh line (`terminalOutputTail`).
fn tail(text: &str, max: usize) -> (String, bool) {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return (text.to_owned(), false);
    }
    let start = chars.len() - max;
    // Drop the partial first line unless that would discard most of the tail.
    if let Some(nl) = (start..chars.len()).find(|&i| chars[i] == '\n') {
        if ((nl - start) as f64) < max as f64 / 4.0 {
            return (chars[nl + 1..].iter().collect(), true);
        }
    }
    // Cutting inside a line: never split an escape, and keep the colour in force.
    let (start, prefix) = style_at(&chars, start);
    (
        format!("{prefix}{}", chars[start..].iter().collect::<String>()),
        true,
    )
}

/// The active style at `index` and where a cut there may safely start.
fn style_at(chars: &[char], index: usize) -> (usize, String) {
    let line_start = (0..index)
        .rev()
        .find(|&i| chars[i] == '\n')
        .map_or(0, |i| i + 1);
    let mut prefix = String::new();
    let mut cursor = line_start;
    while cursor < index {
        let Some(found) = (cursor..chars.len().saturating_sub(1))
            .find(|&i| chars[i] == ESC && chars[i + 1] == '[')
        else {
            break;
        };
        if found >= index {
            break;
        }
        let Some(end) = (found..chars.len()).find(|&i| chars[i] == 'm') else {
            break;
        };
        if end >= index {
            return (end + 1, chars[found..=end].iter().collect());
        }
        prefix = chars[found..=end].iter().collect();
        cursor = end + 1;
    }
    (
        index,
        if prefix == sgr("") {
            String::new()
        } else {
            prefix
        },
    )
}

/// How a span of output is drawn; colours are sRGB bytes already resolved against a palette.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpanStyle {
    pub color: Option<[u8; 3]>,
    pub background: Option<[u8; 3]>,
    pub bold: bool,
    /// Drawn at 65% opacity.
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

/// `TERMINAL_PALETTES`: the 16 ANSI colours for dark and light surfaces.
pub const PALETTE_DARK: [&str; 16] = [
    "#3b4048", "#f47067", "#57ab5a", "#c69026", "#539bf5", "#b083f0", "#39c5cf", "#adbac7",
    "#768390", "#ff938a", "#6bc46d", "#daaa3f", "#6cb6ff", "#dcbdfb", "#56d4dd", "#cdd9e5",
];
pub const PALETTE_LIGHT: [&str; 16] = [
    "#24292f", "#cf222e", "#116329", "#7d4e00", "#0550ae", "#8250df", "#1b7c83", "#6e7781",
    "#57606a", "#a40e26", "#1a7f37", "#633c01", "#0969da", "#6639ba", "#3192aa", "#8c959f",
];

fn hex(c: &str) -> [u8; 3] {
    let v = |o: usize| u8::from_str_radix(&c[o..o + 2], 16).unwrap_or(0);
    [v(1), v(3), v(5)]
}

fn xterm_color(index: i64, palette: &[&str; 16]) -> [u8; 3] {
    if index < 16 {
        return hex(palette[index as usize]);
    }
    if index >= 232 {
        let level = (8 + (index - 232) * 10) as u8;
        return [level; 3];
    }
    let cube = index - 16;
    let channel = |v: i64| if v == 0 { 0 } else { (55 + v * 40) as u8 };
    [channel(cube / 36), channel(cube / 6 % 6), channel(cube % 6)]
}

fn resolve_color(code: &str, base: i64, palette: &[&str; 16]) -> Option<[u8; 3]> {
    let parts: Vec<i64> = code.split(';').map(js_number).collect();
    let head = parts[0];
    if head == base + 8 {
        if parts.get(1) == Some(&5) {
            return Some(xterm_color(parts[2], palette));
        }
        return Some([parts[2] as u8, parts[3] as u8, parts[4] as u8]);
    }
    if (base..=base + 7).contains(&head) {
        return Some(hex(palette[(head - base) as usize]));
    }
    if (base + 60..=base + 67).contains(&head) {
        return Some(hex(palette[(head - base - 52) as usize]));
    }
    None
}

fn luminance(c: [u8; 3]) -> f64 {
    let ch = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
}

fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn span_style(pen: &str, palette: &[&str; 16]) -> Option<SpanStyle> {
    if pen.is_empty() {
        return None;
    }
    let a = parse_pen(pen);
    let mut color = if a.fg.is_empty() {
        None
    } else {
        resolve_color(&a.fg, 30, palette)
    };
    let mut background = if a.bg.is_empty() {
        None
    } else {
        resolve_color(&a.bg, 40, palette)
    };
    if a.inverse {
        (color, background) = (background, Some(color.unwrap_or(hex(palette[7]))));
    }
    // Badges like "PASS" on green assume a terminal's own palette; keep their text readable.
    if let Some(bg) = background {
        if color.is_none_or(|fg| contrast(fg, bg) < 3.0) {
            color = Some(if luminance(bg) > 0.35 {
                hex("#1f2328")
            } else {
                hex("#ffffff")
            });
        }
    }
    Some(SpanStyle {
        color,
        background,
        bold: a.bold,
        dim: a.dim,
        italic: a.italic,
        underline: a.underline,
        strike: a.strike,
    })
}

/// `terminalOutputSpans`: the text split by style, capped at `max_spans` styled spans and then
/// one plain remainder, to bound how much a huge colourful log costs to draw.
pub fn spans(
    text: &str,
    palette: &[&str; 16],
    max_spans: usize,
) -> Vec<(String, Option<SpanStyle>)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<(String, Option<SpanStyle>)> = Vec::new();
    let mut pen = String::new();
    let mut cursor = 0;
    let push = |out: &mut Vec<(String, Option<SpanStyle>)>, from: usize, to: usize, pen: &str| {
        if to > from {
            out.push((chars[from..to].iter().collect(), span_style(pen, palette)));
        }
    };
    while cursor < chars.len() {
        let Some(esc) = (cursor..chars.len().saturating_sub(1))
            .find(|&i| chars[i] == ESC && chars[i + 1] == '[')
        else {
            break;
        };
        if out.len() >= max_spans {
            break;
        }
        push(&mut out, cursor, esc, &pen);
        let Some(end) = (esc..chars.len()).find(|&i| chars[i] == 'm') else {
            break;
        };
        let params: String = chars[esc + 2..end].iter().collect();
        pen = if params == "0" {
            String::new()
        } else {
            params.strip_prefix("0;").unwrap_or(&params).to_owned()
        };
        cursor = end + 1;
    }
    if cursor < chars.len() {
        if out.len() >= max_spans {
            out.push((
                plain_text(&chars[cursor..].iter().collect::<String>()),
                None,
            ));
        } else {
            push(&mut out, cursor, chars.len(), &pen);
        }
    }
    out
}

/// The text with SGR escapes removed.
pub fn plain_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ESC && chars.get(i + 1) == Some(&'[') {
            if let Some(end) =
                (i + 2..chars.len()).find(|&j| !(chars[j].is_ascii_digit() || chars[j] == ';'))
            {
                if chars[end] == 'm' {
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}
