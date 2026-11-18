//! The sync summary of `views.get`: the TUI status line's wording, minus its `:command` hints,
//! for a client that can only inform (the phone). The first thing that needs a person wins.

use chrono::{DateTime, FixedOffset};

use crate::protocol::methods::{SyncView, WireCollState};
use crate::protocol::views::SyncSummary;

/// Longest detail, in characters, so an error text cannot flood a phone.
const DETAIL_MAX: usize = 300;

fn ago(secs: i64) -> String {
    match secs {
        ..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

fn cap(text: String) -> String {
    if text.chars().count() <= DETAIL_MAX {
        return text;
    }
    let mut out: String = text.chars().take(DETAIL_MAX - 1).collect();
    out.push('…');
    out
}

pub fn summary(
    sync: &SyncView,
    conflicts: usize,
    now: DateTime<FixedOffset>,
    list_name: &dyn Fn(&str) -> String,
) -> SyncSummary {
    let attention = |headline: String, detail: String| SyncSummary {
        attention: true,
        headline,
        detail: Some(cap(detail)),
        last_success: sync.last_success,
    };
    let discovery_in_error = sync
        .last_error
        .as_deref()
        .filter(|e| e.starts_with("list discovery failed;"));
    if let Some(err) = sync
        .last_error
        .as_deref()
        .filter(|_| discovery_in_error.is_none())
    {
        let next = sync
            .next_attempt
            .map(|t| {
                format!(
                    " Next try {}.",
                    t.with_timezone(now.offset()).format("%H:%M")
                )
            })
            .unwrap_or_default();
        return attention(
            "Sync failing".into(),
            format!("{}.{next}", err.trim_end_matches('.')),
        );
    }
    if let Some(err) = discovery_in_error.or(sync.discovery_error.as_deref()) {
        return attention("List discovery failed".into(), err.to_string());
    }
    let emptied: Vec<(String, bool)> = sync
        .collections
        .iter()
        .filter_map(|(id, st)| match st {
            WireCollState::EmptiedRemote => Some((list_name(id), true)),
            WireCollState::EmptiedLocal => Some((list_name(id), false)),
            _ => None,
        })
        .collect();
    if !emptied.is_empty() {
        let headline = match emptied.as_slice() {
            [(name, true)] => format!("\"{name}\" was emptied on the server"),
            [(name, false)] => format!("\"{name}\" was emptied here"),
            many => format!("{} lists were emptied", many.len()),
        };
        let runs: Vec<String> = emptied
            .iter()
            .map(|(name, _)| format!("Run: tdo sync --force-delete \"{name}\""))
            .collect();
        let detail = if emptied.len() == 1 {
            format!("Not applied automatically. {}", runs[0])
        } else {
            runs.join("; ")
        };
        return attention(headline, detail);
    }
    if conflicts > 0 {
        let headline = match conflicts {
            1 => "1 conflict".to_string(),
            n => format!("{n} conflicts"),
        };
        return attention(headline, "Resolve them in tdo.".into());
    }
    if !sync.auto {
        return attention("Sync is off".into(), "Turn it on in tdo.".into());
    }
    let headline = match sync.last_success {
        Some(t) => format!("Synced {}", ago((now.to_utc() - t).num_seconds())),
        None => "Not synced yet".into(),
    };
    SyncSummary {
        attention: false,
        headline,
        detail: None,
        last_success: sync.last_success,
    }
}
