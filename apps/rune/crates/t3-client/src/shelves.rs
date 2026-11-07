//! The sidebar's per-thread rules, ported from the web and kept equal to it by
//! `tests/shelves.rs`: snoozing (`effectiveSnoozed` over `presentThreadShell` in
//! `packages/client-runtime/src/state/{threadSettled,models}.ts`), unseen completions
//! (`hasUnseenCompletion` in `apps/web/src/components/Sidebar.logic.ts`) and pinned order
//! (`sortPinnedThreadsByOrderKey` in `state/threadSort.ts`).
use std::cmp::Ordering;

use crate::protocol::ThreadShell;

/// `Date.parse` for the ISO timestamps the server sends, in milliseconds.
pub fn iso_ms(s: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    if s.get(4..5)? != "-" || s.get(7..8)? != "-" {
        return None;
    }
    let (h, mi, se) = match s.get(10..11) {
        None => (0, 0, 0),
        Some("T" | " ") => (n(11..13)?, n(14..16)?, n(17..19).unwrap_or(0)),
        Some(_) => return None,
    };
    let mut rest = s.get(19..).unwrap_or("");
    let mut ms = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.chars().take_while(char::is_ascii_digit).count();
        let take = &frac[..digits.min(3)];
        ms = format!("{take:0<3}").parse::<i64>().ok()?;
        rest = &frac[digits..];
    }
    let offset_min = match rest {
        "" | "Z" | "z" => 0,
        o => {
            let sign = match o.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let digits: String = o[1..].chars().filter(char::is_ascii_digit).collect();
            let hh = digits.get(0..2)?.parse::<i64>().ok()?;
            let mm = digits
                .get(2..4)
                .map_or(Some(0), |m| m.parse::<i64>().ok())?;
            sign * (hh * 60 + mm)
        }
    };
    // Days from civil (Howard Hinnant).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let secs = (era * 146097 + doe - 719468) * 86400 + h * 3600 + mi * 60 + se;
    Some((secs - offset_min * 60) * 1000 + ms)
}

fn terminal(status: &str) -> bool {
    matches!(
        status,
        "completed" | "interrupted" | "failed" | "cancelled" | "rolled_back"
    )
}

/// The latest run's completion time as the web presents it: the server's when it sends one,
/// else the shell's update time once the run has ended.
pub fn latest_run_completed_at(t: &ThreadShell) -> Option<&str> {
    t.latest_run_id.as_ref()?;
    match &t.latest_run_completed_at {
        Some(sent) => sent.as_deref(),
        None if t.status == "idle" || terminal(&t.status) => Some(&t.updated_at),
        None => None,
    }
}

/// `backgroundWorkHoldsCompletion`: subagents and background tasks hold the thread at idle;
/// commands and monitors only when the agent marked them awaited.
fn background_holds(t: &ThreadShell) -> bool {
    t.pending_background_tasks.iter().any(|task| {
        task.awaited == Some(true) || matches!(task.kind.as_str(), "subagent" | "background_task")
    })
}

/// `failureEdgeAt`: `None` when the thread hasn't failed; `Some(None)` when it failed at an
/// unknown time.
fn failure_edge_at(t: &ThreadShell) -> Option<Option<&str>> {
    let park = background_holds(t) && t.status != "failed";
    if t.latest_run_id.is_none() && t.active_provider_thread_id.is_none() && !park {
        return None;
    }
    let status = if park {
        "idle"
    } else {
        t.activity_run_status.as_deref().unwrap_or(&t.status)
    };
    if status != "error" && status != "failed" {
        return None;
    }
    let run_failed = t.latest_run_id.is_some() && t.status == "failed";
    Some(if run_failed {
        latest_run_completed_at(t)
    } else {
        Some(&t.updated_at)
    })
}

fn request_kind(t: &ThreadShell) -> Option<&str> {
    t.pending_runtime_request.as_ref()?.get("kind")?.as_str()
}

/// `threadRaisedHandWhileSnoozed`: a request for the user or a fresh failure ends a snooze
/// early. Subagent requests can't be seen from a shell row, so they don't wake one here.
fn raised_hand(t: &ThreadShell) -> bool {
    let kind = request_kind(t);
    if kind.is_some_and(|k| k != "user_input") {
        return true;
    }
    if kind == Some("user_input") {
        let asked = t
            .pending_runtime_request
            .as_ref()
            .and_then(|r| r.get("createdAt")?.as_str())
            .and_then(iso_ms);
        let snoozed = t.snoozed_at.as_deref().and_then(iso_ms);
        // Snoozing an open question means "ask me later"; only a newer one wakes it.
        match (asked, snoozed, t.snoozed_at.is_some()) {
            (Some(a), Some(s), true) if a <= s => {}
            _ => return true,
        }
    }
    if t.pending_guest_message_count.unwrap_or(0) > 0 {
        return true;
    }
    match failure_edge_at(t) {
        None => false,
        Some(failed) => match t.snoozed_at.as_deref() {
            None => true,
            Some(at) => match (failed.and_then(iso_ms), iso_ms(at)) {
                (Some(f), Some(s)) => f > s,
                _ => false,
            },
        },
    }
}

/// `effectiveSnoozed`: hidden on the Snoozed shelf until `snoozedUntil`, unless it raised its
/// hand first.
pub fn is_snoozed(t: &ThreadShell, now_ms: i64) -> bool {
    let Some(wake) = t.snoozed_until.as_deref().and_then(iso_ms) else {
        return false;
    };
    wake > now_ms && !raised_hand(t)
}

/// `hasUnseenCompletion`: the latest run finished after the user last looked.
pub fn has_unseen_completion(t: &ThreadShell) -> bool {
    let Some(done) = latest_run_completed_at(t).and_then(iso_ms) else {
        return false;
    };
    let Some(Some(visited)) = &t.last_visited_at else {
        return false;
    };
    match iso_ms(visited) {
        Some(v) => done > v,
        None => true,
    }
}

/// `sortPinnedThreadsByOrderKey`: keyed pins by key, then the rest newest first; ties by id,
/// then environment.
pub fn sort_pinned(threads: &mut [(&str, &ThreadShell)]) {
    let tie = |a: &(&str, &ThreadShell), b: &(&str, &ThreadShell)| {
        a.1.id.cmp(&b.1.id).then_with(|| a.0.cmp(b.0))
    };
    threads.sort_by(
        |a, b| match (a.1.pin_order_key.as_deref(), b.1.pin_order_key.as_deref()) {
            (Some(x), Some(y)) => x.cmp(y).then_with(|| tie(a, b)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => {
                let at = |t: &ThreadShell| iso_ms(&t.created_at).unwrap_or(0);
                at(b.1).cmp(&at(a.1)).then_with(|| tie(a, b))
            }
        },
    );
}
