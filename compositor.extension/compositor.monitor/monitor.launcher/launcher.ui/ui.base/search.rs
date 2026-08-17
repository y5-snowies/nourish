//! Search and ranking.
//!
//! Two-stage scoring:
//!
//! 1. **Match score** — how well the query matches the application.
//!    Ported from the user's TypeScript reference. The tree's
//!    "path match" rule is reused by treating `(title, bin_filename)`
//!    as a two-segment path: "doce" can match "Example" via the
//!    title, and "ff" can match "firefox" via the binary name even
//!    when the title is "Mozilla Firefox".
//!
//! 2. **Usage score** — a recency-weighted frequency score, à la
//!    Firefox frecency. Combined with the match score for search
//!    ranking, used alone for the default (empty-query) sort.

use std::time::SystemTime;

use crate::model::Application;

/// Normalise a string the way the TS reference does: lowercase, then
/// strip every non-alphanumeric character.
fn normalise(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// Subsequence check used by tests. Production code goes through
/// [`subsequence_score`] which is strictly more informative.
#[cfg(test)]
fn is_subsequence(text_norm: &str, query_norm: &str) -> bool {
    if query_norm.is_empty() {
        return true;
    }
    let mut q = query_norm.bytes();
    let mut next = q.next();
    for b in text_norm.bytes() {
        if Some(b) == next {
            next = q.next();
            if next.is_none() {
                return true;
            }
        }
    }
    next.is_none()
}

/// Score how well `query` matches `text` as a subsequence. Higher is
/// better. Roughly in [0.0, 1.3].
///
/// Rewards:
/// - contiguous runs (so "fir" beats "fxr" against "firefox")
/// - matches anchored near the start
/// - shorter target strings (so "vim" beats "neovim-qt" against "vim")
///
/// Returns `None` if `query` is not a subsequence of `text`.
fn subsequence_score(text: &str, query_norm: &str) -> Option<f32> {
    let text_norm = normalise(text);
    if query_norm.is_empty() {
        return Some(0.0);
    }
    if text_norm.is_empty() {
        return None;
    }

    let tb = text_norm.as_bytes();
    let qb = query_norm.as_bytes();

    let mut qi = 0usize;
    let mut score = 0.0f32;
    let mut run = 0u32;
    let mut first_match: Option<usize> = None;

    for (i, &c) in tb.iter().enumerate() {
        if qi < qb.len() && c == qb[qi] {
            if first_match.is_none() {
                first_match = Some(i);
            }
            qi += 1;
            run += 1;
            // Quadratic bonus for contiguous runs: 1, 4, 9, 16…
            score += (run * run) as f32;
        } else {
            run = 0;
        }
    }

    if qi != qb.len() {
        return None;
    }

    // Normalise by the maximum possible run score (entire query
    // contiguous).
    let q_len = qb.len() as f32;
    let max_run_score = q_len * q_len;
    let mut s = score / max_run_score;

    // Anchor bonus: matching at position 0 is worth a small extra nudge.
    if let Some(pos) = first_match {
        s += 0.10 * (1.0 / (1.0 + pos as f32));
    }

    // Length bonus: shorter haystacks for the same query are better.
    let len_bonus = q_len / tb.len() as f32;
    s += 0.15 * len_bonus;

    Some(s)
}

/// Filename portion of a path-like binary string.
fn bin_filename(bin: &std::path::Path) -> String {
    bin.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Pure match score for `(app, query)` — `None` if no match.
///
/// Rules, in order:
///
/// - **Rule 1 (direct title hit):** query is a subsequence of `title`.
/// - **Rule 2 (cross-field path hit):** query is a subsequence of the
///   binary's filename. Dampened slightly so title matches win ties.
///
/// Returns the higher of the two scores when both apply.
pub fn match_score(app: &Application, query_norm: &str) -> Option<f32> {
    if query_norm.is_empty() {
        return Some(0.0);
    }

    let title_score = subsequence_score(&app.title, query_norm);

    // Match against the MAIN entry's binary. Actions of the same app almost
    // always share it, and searching the action list too would let "New
    // Window" pull in every app that happens to declare one.
    let bin_name = app
        .entries
        .first()
        .map(|e| bin_filename(&e.bin))
        .unwrap_or_default();
    let bin_score = if bin_name.is_empty() {
        None
    } else {
        // 0.85 dampens bin-only matches a bit so title matches naturally
        // win ties.
        subsequence_score(&bin_name, query_norm).map(|s| s * 0.85)
    };

    match (title_score, bin_score) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Frecency-style usage score in [0.0, 1.0].
///
/// Log-compressed frequency combined with a 14-day-halflife
/// exponential recency decay.
pub fn usage_score(app: &Application, now: SystemTime) -> f32 {
    const HALFLIFE_SECS: f32 = 14.0 * 24.0 * 3600.0;

    // ln(403) ≈ 6 → caps usefulness at ~400 uses.
    let freq = (app.usage_count as f32 + 1.0).ln() / 6.0;
    let freq = freq.clamp(0.0, 1.0);

    let recency = match app.usage_time {
        None => 0.0,
        Some(t) => {
            let dt = now
                .duration_since(t)
                .map(|d| d.as_secs_f32())
                .unwrap_or(0.0);
            0.5f32.powf(dt / HALFLIFE_SECS).clamp(0.0, 1.0)
        }
    };

    0.4 * freq + 0.6 * recency
}

/// Combined ranking score used when search is active. Match quality
/// dominates; usage breaks ties.
fn combined_score(match_s: f32, usage_s: f32) -> f32 {
    match_s + 0.15 * usage_s
}

/// Sort the full application list for the default (empty-query) view.
/// Returns indices into `apps` in display order.
pub fn rank_default(apps: &[Application], now: SystemTime) -> Vec<usize> {
    let mut scored: Vec<(usize, f32)> = apps
        .iter()
        .enumerate()
        .map(|(i, a)| (i, usage_score(a, now)))
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    scored.into_iter().map(|(i, _)| i).collect()
}

/// Search and rank. Empty/whitespace-only/symbol-only queries fall
/// through to [`rank_default`].
pub fn search(apps: &[Application], query: &str, now: SystemTime) -> Vec<usize> {
    let q = query.trim();
    if q.is_empty() {
        return rank_default(apps, now);
    }
    let q_norm = normalise(q);
    if q_norm.is_empty() {
        return rank_default(apps, now);
    }

    let mut hits: Vec<(usize, f32)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let m = match_score(a, &q_norm)?;
            let u = usage_score(a, now);
            Some((i, combined_score(m, u)))
        })
        .collect();

    hits.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    hits.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    fn app(id: &str, title: &str, bin: &str, count: u64, age_days: Option<f32>) -> Application {
        Application {
            id: id.into(),
            title: title.into(),
            entries: vec![crate::model::AppEntry {
                action: None,
                title: title.into(),
                bin: PathBuf::from(bin),
                args: vec![],
                icon_path: None,
            }],
            default_entry: 0,
            usage_count: count,
            usage_time: age_days
                .map(|d| SystemTime::now() - Duration::from_secs_f32(d * 86400.0)),
        }
    }

    #[test]
    fn subsequence_basics() {
        assert!(is_subsequence("firefox", "ff"));
        assert!(is_subsequence("firefox", "fox"));
        assert!(!is_subsequence("firefox", "xff"));
        assert!(is_subsequence("anything", ""));
    }

    #[test]
    fn rule1_direct_title_hit() {
        let apps = vec![
            app("doc1", "Document 1", "/usr/bin/doc1", 0, None),
            app("doc2", "Document 2", "/usr/bin/doc2", 0, None),
        ];
        let r = search(&apps, "Doc", SystemTime::now());
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn rule2_cross_field_match() {
        let apps = vec![
            app("ff", "Mozilla Firefox", "/usr/bin/firefox", 0, None),
            app("term", "Terminal", "/usr/bin/xterm", 0, None),
        ];
        let r = search(&apps, "ff", SystemTime::now());
        assert_eq!(r.first().copied(), Some(0));
    }

    #[test]
    fn usage_score_recency_decay() {
        let now = SystemTime::now();
        let fresh = app("a", "A", "/a", 5, Some(0.0));
        let stale = app("b", "B", "/b", 5, Some(60.0));
        assert!(usage_score(&fresh, now) > usage_score(&stale, now));
    }

    #[test]
    fn default_sort_prefers_frecent() {
        let apps = vec![
            app("never", "Never", "/n", 0, None),
            app("freq_old", "FreqOld", "/fo", 50, Some(120.0)),
            app("freq_new", "FreqNew", "/fn", 50, Some(0.1)),
        ];
        let r = rank_default(&apps, SystemTime::now());
        assert_eq!(r[0], 2);
        assert_eq!(r[2], 0);
    }

    #[test]
    fn search_breaks_ties_by_usage() {
        let apps = vec![
            app("vim_old", "vim", "/usr/bin/vim", 0, None),
            app("vim_new", "vim", "/usr/local/bin/vim", 10, Some(0.0)),
        ];
        let r = search(&apps, "vim", SystemTime::now());
        assert_eq!(r[0], 1);
    }

    #[test]
    fn empty_query_falls_through() {
        let apps = vec![app("a", "A", "/a", 1, Some(0.0))];
        assert_eq!(search(&apps, "   ", SystemTime::now()), vec![0]);
        assert_eq!(search(&apps, "!!!", SystemTime::now()), vec![0]);
    }
}
