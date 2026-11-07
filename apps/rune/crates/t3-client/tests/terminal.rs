//! Replays tests/fixtures/terminal.json, produced by the web's real `appendTerminalOutput` and
//! `terminalOutputSpans` (`node apps/rune/scripts/terminal-reference.ts`).
use serde_json::Value;
use t3_client::terminal::{PALETTE_DARK, SpanStyle, TerminalOutput, spans};

/// A CSS colour as the TypeScript writes them: `#rrggbb` or `rgb(r,g,b)`.
fn css(c: &str) -> [u8; 3] {
    if let Some(h) = c.strip_prefix('#') {
        let v = |o: usize| u8::from_str_radix(&h[o..o + 2], 16).unwrap();
        return [v(0), v(2), v(4)];
    }
    let n: Vec<u8> = c
        .trim_start_matches("rgb(")
        .trim_end_matches(')')
        .split(',')
        .map(|p| p.parse().unwrap())
        .collect();
    [n[0], n[1], n[2]]
}

fn style(v: &Value) -> Option<SpanStyle> {
    let o = v.as_object()?;
    let deco = o
        .get("textDecorationLine")
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(SpanStyle {
        color: o.get("color").and_then(Value::as_str).map(css),
        background: o.get("backgroundColor").and_then(Value::as_str).map(css),
        bold: o.get("fontWeight").is_some(),
        dim: o.get("opacity").is_some(),
        italic: o.get("fontStyle").is_some(),
        underline: deco.contains("underline"),
        strike: deco.contains("line-through"),
    })
}

#[test]
fn command_output_folds_like_the_web() {
    let fixture: Vec<Value> = serde_json::from_str(include_str!("fixtures/terminal.json")).unwrap();
    let mut failures = Vec::new();
    for case in &fixture {
        let name = case["name"].as_str().unwrap();
        let mut out = TerminalOutput::with_max(case["max"].as_u64().unwrap() as usize);
        for chunk in case["chunks"].as_array().unwrap() {
            out.append(chunk.as_str().unwrap());
        }
        if out.text != case["text"].as_str().unwrap() {
            failures.push(format!(
                "{name}: text\n  rust {:?}\n  web  {:?}",
                out.text,
                case["text"].as_str().unwrap()
            ));
            continue;
        }
        if out.truncated != case["truncated"].as_bool().unwrap() {
            failures.push(format!(
                "{name}: truncated {} != {}",
                out.truncated, case["truncated"]
            ));
        }
        if out.pending() != case["pending"].as_str().unwrap() {
            failures.push(format!(
                "{name}: pending {:?} != {:?}",
                out.pending(),
                case["pending"]
            ));
        }
        let want: Vec<(String, Option<SpanStyle>)> = case["spans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s[0].as_str().unwrap().to_owned(), style(&s[1])))
            .collect();
        let got = spans(&out.text, &PALETTE_DARK, 2_000);
        if got != want {
            failures.push(format!("{name}: spans\n  rust {got:?}\n  web  {want:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        fixture.len(),
        failures.join("\n")
    );
}

#[test]
fn a_replace_frame_starts_over_and_append_frames_continue() {
    let mut out = TerminalOutput::default();
    out.apply_frame(false, "one\r", false);
    out.apply_frame(false, "two\n", false);
    assert_eq!(out.text, "two\n");
    out.apply_frame(true, "fresh", true);
    assert_eq!((out.text.as_str(), out.truncated), ("fresh", true));
}
