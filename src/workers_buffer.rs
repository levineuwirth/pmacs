// workers_buffer.rs --- T M3.7 *workers* observability buffer renderer.

//! `*workers*` buffer rendering ([T M3.7]).
//!
//! This module is the Rust side of the observability buffer the spec
//! calls out: a live view of active jobs, the supersede table, and a
//! recent-completions ring. It mirrors the renderer pattern in
//! [`crate::help`] --- a [`crate::async_runtime::WorkersSnapshot`]
//! goes in, formatted text replaces the buffer's contents, the
//! buffer is marked clean.
//!
//! # Buffer layout
//!
//! ```text
//! Workers (active: 2, completed: 5)
//!
//! ID      Kind         Age       Supersede   Status
//! ------  -----------  --------  ----------  ----------
//! #5      grep         412ms     search      running
//! #6      sleep        18ms                  running (cancel pending)
//!
//! Recent (newest first)
//!
//! ID      Kind         Duration  Supersede   Outcome
//! ------  -----------  --------  ----------  ----------
//! #4      grep         1242ms    search      cancelled (3s ago)
//! #3      compute_sum  2ms                   ok (3s ago)
//! ```
//!
//! Lua reads the snapshot via `pmacs.workers.snapshot()`; the
//! `pmacs.workers.show()` builtin invokes [`render`] on it and
//! returns the buffer id. Auto-refresh hooks into
//! `pmacs._async.tick`.

use std::fmt::Write;

use crate::async_runtime::{
    ActiveJobInfo, CompletedJobInfo, JobOutcome, JobResult, WorkersSnapshot,
};
use crate::buffer::{BufferId, EditOp};
use crate::buffer_registry::BufferRegistry;

/// Canonical name for the workers observability buffer.
pub const WORKERS_BUFFER_NAME: &str = "*workers*";

/// Render `snapshot` into the `*workers*` buffer (creating it if
/// absent), replacing its full contents. Returns the buffer id.
///
/// The buffer is marked clean after rendering --- the modeline
/// shouldn't claim unsaved changes for a generated buffer.
pub fn render(registry: &mut BufferRegistry, snapshot: &WorkersSnapshot) -> BufferId {
    let text = format_snapshot(snapshot);
    let id = registry
        .find_by_name(WORKERS_BUFFER_NAME)
        .unwrap_or_else(|| registry.create(WORKERS_BUFFER_NAME));
    let buf = registry.get_mut(id).expect("just resolved");
    if !buf.is_empty() {
        let len = buf.len();
        let _ = buf.apply_edit(EditOp::Delete {
            range: crate::rope::Range::new(0, len),
        });
    }
    if !text.is_empty() {
        let _ = buf.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: text.as_bytes(),
        });
    }
    buf.mark_clean();
    id
}

/// Format a snapshot as the buffer's text payload.
fn format_snapshot(snapshot: &WorkersSnapshot) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "Workers (active: {}, completed: {})",
        snapshot.active.len(),
        snapshot.completed.len()
    );
    let _ = writeln!(text);
    let _ = writeln!(
        text,
        "{:<7} {:<11} {:>9} {:<11} Status",
        "ID", "Kind", "Age", "Supersede"
    );
    let _ = writeln!(
        text,
        "{:<7} {:<11} {:>9} {:<11} ----------",
        "------", "-----------", "---------", "-----------"
    );
    if snapshot.active.is_empty() {
        let _ = writeln!(text, "(no active jobs)");
    } else {
        for job in &snapshot.active {
            write_active_row(&mut text, job);
        }
    }
    let _ = writeln!(text);
    let _ = writeln!(text, "Recent (newest first)");
    let _ = writeln!(text);
    let _ = writeln!(
        text,
        "{:<7} {:<11} {:>9} {:<11} Outcome",
        "ID", "Kind", "Duration", "Supersede"
    );
    let _ = writeln!(
        text,
        "{:<7} {:<11} {:>9} {:<11} ----------",
        "------", "-----------", "---------", "-----------"
    );
    if snapshot.completed.is_empty() {
        let _ = writeln!(text, "(no recent completions)");
    } else {
        for job in &snapshot.completed {
            write_completed_row(&mut text, job);
        }
    }
    text
}

fn write_active_row(text: &mut String, job: &ActiveJobInfo) {
    let id = format!("#{}", job.id);
    let kind = job.kind.label();
    let age = format_duration_ms(job.age_ms);
    let key = job.supersede_key.as_deref().unwrap_or("");
    let mut status = String::from("running");
    if job.cancel_requested {
        status.push_str(" (cancel pending)");
    }
    if job.is_stream {
        status.push_str(" [stream]");
    }
    let _ = writeln!(text, "{id:<7} {kind:<11} {age:>9} {key:<11} {status}");
}

fn write_completed_row(text: &mut String, job: &CompletedJobInfo) {
    let id = format!("#{}", job.id);
    let kind = job.kind.label();
    let duration = format_duration_ms(job.duration_ms);
    let key = job.supersede_key.as_deref().unwrap_or("");
    let outcome = format_outcome(&job.outcome);
    let age = format_duration_ms(job.settled_age_ms);
    let _ = writeln!(
        text,
        "{id:<7} {kind:<11} {duration:>9} {key:<11} {outcome} ({age} ago)"
    );
}

fn format_duration_ms(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m{:02}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

fn format_outcome(outcome: &JobOutcome) -> String {
    match outcome {
        JobOutcome::Complete(JobResult::Unit) => "ok".to_string(),
        JobOutcome::Complete(JobResult::Sum(v)) => format!("ok (sum={v})"),
        JobOutcome::Complete(JobResult::Parse { duration_ms }) => {
            format!("ok (parse {duration_ms}ms)")
        }
        JobOutcome::Cancelled => "cancelled".to_string(),
        JobOutcome::Failed(msg) => {
            // Trim the failure message for the table; the full
            // message is still visible in the buffer's surrounding
            // text once the user follows up with `describe-job`
            // (M4 follow-up).
            let head: String = msg.chars().take(40).collect();
            format!("failed: {head}")
        }
    }
}

/// Locate the job id at byte position `pos` within the rendered
/// `*workers*` buffer. The line at that position must start with
/// `#<digits>`; whitespace and arbitrary commentary after the id are
/// ignored. Returns `None` if the cursor isn't on a row that names a
/// job (e.g. the header, separator, or "(no active jobs)" lines).
///
/// This is what the buffer-local cancel binding uses to translate
/// "the cursor is here, please cancel" into a [`JobId`].
#[must_use]
pub fn job_id_at_byte(text: &str, pos: usize) -> Option<u64> {
    let pos = pos.min(text.len());
    // Find the start of the line containing `pos`.
    let line_start = text[..pos].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);
    let line = &text[line_start..line_end];
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('#')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::async_runtime::JobKind;

    fn snapshot_with(
        active: Vec<ActiveJobInfo>,
        completed: Vec<CompletedJobInfo>,
    ) -> WorkersSnapshot {
        WorkersSnapshot { active, completed }
    }

    #[test]
    fn empty_snapshot_renders_placeholder_lines() {
        let s = snapshot_with(vec![], vec![]);
        let text = format_snapshot(&s);
        assert!(text.contains("(no active jobs)"));
        assert!(text.contains("(no recent completions)"));
    }

    #[test]
    fn active_row_includes_id_kind_age_status() {
        let s = snapshot_with(
            vec![ActiveJobInfo {
                id: 7,
                kind: JobKind::Grep,
                age_ms: 412,
                supersede_key: Some("search".to_string()),
                cancel_requested: false,
                is_stream: true,
            }],
            vec![],
        );
        let text = format_snapshot(&s);
        assert!(text.contains("#7"), "id missing: {text}");
        assert!(text.contains("grep"));
        assert!(text.contains("412ms"));
        assert!(text.contains("search"));
        assert!(text.contains("running"));
        assert!(text.contains("[stream]"));
    }

    #[test]
    fn cancel_pending_row_marks_pending() {
        let s = snapshot_with(
            vec![ActiveJobInfo {
                id: 1,
                kind: JobKind::Sleep,
                age_ms: 5,
                supersede_key: None,
                cancel_requested: true,
                is_stream: false,
            }],
            vec![],
        );
        let text = format_snapshot(&s);
        assert!(text.contains("(cancel pending)"));
    }

    #[test]
    fn completed_row_renders_outcome_and_age() {
        let s = snapshot_with(
            vec![],
            vec![CompletedJobInfo {
                id: 3,
                kind: JobKind::ComputeSum,
                duration_ms: 25,
                settled_age_ms: 200,
                supersede_key: None,
                outcome: JobOutcome::Complete(JobResult::Sum(55)),
            }],
        );
        let text = format_snapshot(&s);
        assert!(text.contains("#3"));
        assert!(text.contains("ok (sum=55)"));
        assert!(text.contains("200ms ago"));
    }

    #[test]
    fn duration_formatter_handles_three_scales() {
        assert_eq!(format_duration_ms(42), "42ms");
        assert_eq!(format_duration_ms(1500), "1.5s");
        assert_eq!(format_duration_ms(125_000), "2m05s");
    }

    #[test]
    fn job_id_at_byte_extracts_id_from_active_row() {
        let s = snapshot_with(
            vec![ActiveJobInfo {
                id: 42,
                kind: JobKind::Grep,
                age_ms: 100,
                supersede_key: None,
                cancel_requested: false,
                is_stream: true,
            }],
            vec![],
        );
        let text = format_snapshot(&s);
        // Place cursor inside the data row containing `#42`.
        let pos = text.find("#42").expect("row exists");
        assert_eq!(job_id_at_byte(&text, pos), Some(42));
        // Cursor on the header row produces None.
        let header_pos = text.find("ID ").expect("header exists");
        assert_eq!(job_id_at_byte(&text, header_pos), None);
    }
}
