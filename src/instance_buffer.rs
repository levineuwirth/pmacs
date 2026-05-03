// instance_buffer.rs --- T M5.6f *pmacs-instance* observability buffer renderer.

//! `*pmacs-instance*` buffer rendering ([T M5.6f]).
//!
//! The Rust side of the `editor.describe-instance` and
//! `editor.describe-instance-buffer` commands. Mirrors the renderer
//! pattern in [`crate::workers_buffer`]: a description goes in,
//! formatted text replaces the buffer's contents, the buffer is marked
//! clean.
//!
//! # Two surfaces
//!
//! * [`format_echo_line`] returns a single-line summary suitable for
//!   the status line. Used by `editor.describe-instance`.
//! * [`render`] writes a multi-section detail view into the
//!   `*pmacs-instance*` buffer. Used by
//!   `editor.describe-instance-buffer`.
//!
//! Both consume an [`InstanceIdentity`] for the running process plus
//! an optional [`AttachmentHandle`]. In v0.1 Local mode the attachment
//! is always `None` (the process is its own instance), so the
//! formatters degrade gracefully to "describe self." Once the v0.2
//! reconnect / outbound attach path lands, the same formatters work
//! for the attached case without changes.

use std::fmt::Write;

use crate::buffer::{BufferId, EditOp};
use crate::buffer_registry::BufferRegistry;
use crate::protocol::{AttachmentHandle, InstanceIdentity};

/// Canonical name for the instance description buffer.
pub const INSTANCE_BUFFER_NAME: &str = "*pmacs-instance*";

/// One-line summary for the status row.
///
/// Two shapes, depending on whether `attachment` is set:
///
/// * `None`: `pmacs <ver> [<name>]: <cwd> (uptime <duration>)` — the
///   running process describes itself.
/// * `Some(_)`: `pmacs <ver> attached to <attached_name> (<target>)
///   from <cwd>, uptime <duration>` — the running process is talking
///   to a remote instance whose identity it has captured.
#[must_use]
pub fn format_echo_line(
    identity: &InstanceIdentity,
    attachment: Option<&AttachmentHandle>,
) -> String {
    let uptime = format_uptime(identity.uptime_secs);
    match attachment {
        None => {
            let name = identity.instance_name.as_deref().unwrap_or("local");
            format!(
                "pmacs {ver} [{name}]: {cwd} (uptime {uptime})",
                ver = identity.pmacs_version,
                cwd = identity.working_directory,
            )
        }
        Some(h) => {
            let attached_name = h.identity.instance_name.as_deref().unwrap_or("anonymous");
            format!(
                "pmacs {ver} attached to {attached_name} ({target}) from {cwd}, uptime {uptime}",
                ver = identity.pmacs_version,
                target = h.target,
                cwd = identity.working_directory,
            )
        }
    }
}

/// Render the full description into the `*pmacs-instance*` buffer
/// (creating it if absent), replacing its full contents. Returns the
/// buffer id.
///
/// The buffer is marked clean — the modeline shouldn't claim unsaved
/// changes for a generated buffer.
pub fn render(
    registry: &mut BufferRegistry,
    identity: &InstanceIdentity,
    attachment: Option<&AttachmentHandle>,
) -> BufferId {
    let text = format_full_text(identity, attachment);
    let id = registry
        .find_by_name(INSTANCE_BUFFER_NAME)
        .unwrap_or_else(|| registry.create(INSTANCE_BUFFER_NAME));
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

/// Multi-section text payload for the buffer view. Sections:
///
/// 1. **This instance** — version, build hash, name, cwd, uptime.
/// 2. **Attachment** — only when `attachment` is `Some`. Shows the
///    target string and the remote identity.
/// 3. **Notes** — short hints about how to dismiss the buffer
///    (`q` keybinding) and about the v0.1 single-instance mode.
#[must_use]
pub fn format_full_text(
    identity: &InstanceIdentity,
    attachment: Option<&AttachmentHandle>,
) -> String {
    let mut text = String::new();

    let _ = writeln!(text, "This instance");
    let _ = writeln!(text, "  version:           {}", identity.pmacs_version);
    let _ = writeln!(
        text,
        "  build hash:        {}",
        identity.build_hash.as_deref().unwrap_or("(unknown)")
    );
    let _ = writeln!(
        text,
        "  instance name:     {}",
        identity.instance_name.as_deref().unwrap_or("(unnamed)")
    );
    let _ = writeln!(text, "  working directory: {}", identity.working_directory);
    let _ = writeln!(
        text,
        "  uptime:            {} ({} seconds)",
        format_uptime(identity.uptime_secs),
        identity.uptime_secs
    );

    let _ = writeln!(text);

    match attachment {
        None => {
            let _ = writeln!(text, "Attachment");
            let _ = writeln!(
                text,
                "  (no outbound attachment — this process is its own instance)"
            );
        }
        Some(h) => {
            let _ = writeln!(text, "Attachment");
            let _ = writeln!(text, "  frontend id:       {}", h.frontend_id.0);
            let _ = writeln!(text, "  target:            {}", h.target);
            let _ = writeln!(text, "  kind:              {}", h.target.kind_name());
            let _ = writeln!(text, "  remote version:    {}", h.identity.pmacs_version);
            let _ = writeln!(
                text,
                "  remote name:       {}",
                h.identity.instance_name.as_deref().unwrap_or("(unnamed)")
            );
            let _ = writeln!(
                text,
                "  remote cwd:        {}",
                h.identity.working_directory
            );
            let _ = writeln!(
                text,
                "  remote uptime:     {} ({} seconds)",
                format_uptime(h.identity.uptime_secs),
                h.identity.uptime_secs
            );
            if let Some(hash) = h.identity.build_hash.as_deref() {
                let _ = writeln!(text, "  remote build hash: {hash}");
            }
        }
    }

    let _ = writeln!(text);
    let _ = writeln!(text, "Notes");
    let _ = writeln!(text, "  press `q` to kill this buffer.");
    let _ = writeln!(
        text,
        "  the buffer is regenerated each time you run `editor.describe-instance-buffer`."
    );

    text
}

/// Render `seconds` as a short human-readable duration. Mirrors the
/// scale buckets in `crate::workers_buffer::format_duration_ms` but in
/// seconds (uptimes are rarely sub-second-interesting).
fn format_uptime(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    } else if seconds < 86_400 {
        let hours = seconds / 3_600;
        let minutes = (seconds % 3_600) / 60;
        format!("{hours}h{minutes:02}m")
    } else {
        let days = seconds / 86_400;
        let hours = (seconds % 86_400) / 3_600;
        format!("{days}d{hours:02}h")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AttachTarget, FrontendId, InstanceIdentity};
    use std::path::PathBuf;

    fn local_identity() -> InstanceIdentity {
        InstanceIdentity {
            pmacs_version: "0.1.0".into(),
            build_hash: Some("abc123".into()),
            instance_name: None,
            uptime_secs: 47,
            working_directory: "/tmp/proj".into(),
        }
    }

    fn named_identity() -> InstanceIdentity {
        InstanceIdentity {
            pmacs_version: "0.1.0".into(),
            build_hash: None,
            instance_name: Some("work".into()),
            uptime_secs: 4_500,
            working_directory: "/srv/www".into(),
        }
    }

    fn sample_attachment() -> AttachmentHandle {
        AttachmentHandle::new(
            FrontendId(2),
            named_identity(),
            AttachTarget::LocalSocket(PathBuf::from("/run/pmacs/work.sock")),
        )
    }

    // ---- format_echo_line ---------------------------------------------------

    #[test]
    fn echo_unattached_shows_local_name_when_no_instance_name() {
        let line = format_echo_line(&local_identity(), None);
        assert!(line.starts_with("pmacs 0.1.0 [local]"), "{line}");
        assert!(line.contains("/tmp/proj"));
        assert!(line.contains("uptime 47s"));
    }

    #[test]
    fn echo_unattached_uses_instance_name_when_present() {
        let line = format_echo_line(&named_identity(), None);
        assert!(line.starts_with("pmacs 0.1.0 [work]"), "{line}");
        assert!(line.contains("/srv/www"));
        // 4500 seconds → 1h15m.
        assert!(line.contains("uptime 1h15m"), "{line}");
    }

    #[test]
    fn echo_attached_shows_remote_name_and_target() {
        let h = sample_attachment();
        let line = format_echo_line(&local_identity(), Some(&h));
        assert!(line.contains("attached to work"), "{line}");
        assert!(line.contains("local:/run/pmacs/work.sock"), "{line}");
        assert!(line.contains("from /tmp/proj"));
    }

    #[test]
    fn echo_attached_falls_back_to_anonymous_when_remote_name_missing() {
        let mut id = named_identity();
        id.instance_name = None;
        let h = AttachmentHandle::new(
            FrontendId(2),
            id,
            AttachTarget::LocalSocket(PathBuf::from("/x")),
        );
        let line = format_echo_line(&local_identity(), Some(&h));
        assert!(line.contains("attached to anonymous"), "{line}");
    }

    #[test]
    fn echo_line_is_single_line() {
        let line = format_echo_line(&local_identity(), None);
        assert!(!line.contains('\n'), "echo line must not contain newlines");
        let h = sample_attachment();
        let line = format_echo_line(&local_identity(), Some(&h));
        assert!(!line.contains('\n'), "echo line must not contain newlines");
    }

    // ---- format_full_text ---------------------------------------------------

    #[test]
    fn full_text_unattached_includes_self_section_and_no_attachment_marker() {
        let text = format_full_text(&local_identity(), None);
        assert!(text.contains("This instance"));
        assert!(text.contains("version:           0.1.0"));
        assert!(text.contains("build hash:        abc123"));
        assert!(text.contains("instance name:     (unnamed)"));
        assert!(text.contains("working directory: /tmp/proj"));
        assert!(text.contains("uptime:"));
        assert!(text.contains("(no outbound attachment"));
        assert!(text.contains("Notes"));
        assert!(text.contains("press `q` to kill"));
    }

    #[test]
    fn full_text_unnamed_build_hash_renders_as_unknown() {
        let text = format_full_text(&named_identity(), None);
        assert!(text.contains("build hash:        (unknown)"));
        assert!(text.contains("instance name:     work"));
    }

    #[test]
    fn full_text_attached_renders_remote_section() {
        let h = sample_attachment();
        let text = format_full_text(&local_identity(), Some(&h));
        assert!(text.contains("Attachment"));
        assert!(text.contains("frontend id:       2"));
        assert!(text.contains("target:            local:/run/pmacs/work.sock"));
        assert!(text.contains("kind:              local"));
        assert!(text.contains("remote version:    0.1.0"));
        assert!(text.contains("remote name:       work"));
        assert!(text.contains("remote cwd:        /srv/www"));
        assert!(text.contains("remote uptime:     1h15m (4500 seconds)"));
        // build_hash on the remote is None, so the line should be omitted.
        assert!(
            !text.contains("remote build hash:"),
            "expected no build-hash line for None, got: {text}"
        );
    }

    #[test]
    fn full_text_attached_includes_remote_build_hash_when_present() {
        let mut id = named_identity();
        id.build_hash = Some("def456".into());
        let h = AttachmentHandle::new(
            FrontendId(3),
            id,
            AttachTarget::LocalSocket(PathBuf::from("/x")),
        );
        let text = format_full_text(&local_identity(), Some(&h));
        assert!(text.contains("remote build hash: def456"));
    }

    // ---- format_uptime ------------------------------------------------------

    #[test]
    fn format_uptime_buckets() {
        assert_eq!(format_uptime(0), "0s");
        assert_eq!(format_uptime(47), "47s");
        assert_eq!(format_uptime(60), "1m00s");
        assert_eq!(format_uptime(125), "2m05s");
        assert_eq!(format_uptime(3_600), "1h00m");
        assert_eq!(format_uptime(4_500), "1h15m");
        assert_eq!(format_uptime(86_400), "1d00h");
        assert_eq!(format_uptime(90_000), "1d01h");
    }

    // ---- render -------------------------------------------------------------

    #[test]
    fn render_creates_named_buffer_and_writes_text() {
        let mut reg = BufferRegistry::new();
        let id = render(&mut reg, &local_identity(), None);
        let buf = reg.get(id).expect("just rendered");
        assert_eq!(buf.name(), INSTANCE_BUFFER_NAME);
        let len = buf.len();
        let mut bytes = vec![0u8; usize::try_from(len).unwrap()];
        if len > 0 {
            buf.snapshot_rope().slice(0, len, &mut bytes);
        }
        let body = String::from_utf8(bytes).unwrap();
        assert!(body.contains("This instance"));
        assert!(body.contains("version:           0.1.0"));
        assert!(body.contains("(no outbound attachment"));
        assert!(!buf.is_modified(), "rendered buffer must be marked clean");
    }

    #[test]
    fn render_replaces_existing_contents_on_second_call() {
        let mut reg = BufferRegistry::new();
        let id1 = render(&mut reg, &local_identity(), None);
        let id2 = render(&mut reg, &named_identity(), None);
        assert_eq!(id1, id2, "render must reuse the named buffer");
        let buf = reg.get(id2).expect("rendered");
        let len = buf.len();
        let mut bytes = vec![0u8; usize::try_from(len).unwrap()];
        if len > 0 {
            buf.snapshot_rope().slice(0, len, &mut bytes);
        }
        let body = String::from_utf8(bytes).unwrap();
        assert!(body.contains("instance name:     work"));
        assert!(
            !body.contains("instance name:     (unnamed)"),
            "stale text from first render leaked: {body}"
        );
    }

    #[test]
    fn render_with_attachment_includes_remote_section() {
        let mut reg = BufferRegistry::new();
        let h = sample_attachment();
        let id = render(&mut reg, &local_identity(), Some(&h));
        let buf = reg.get(id).expect("rendered");
        let len = buf.len();
        let mut bytes = vec![0u8; usize::try_from(len).unwrap()];
        if len > 0 {
            buf.snapshot_rope().slice(0, len, &mut bytes);
        }
        let body = String::from_utf8(bytes).unwrap();
        assert!(body.contains("frontend id:       2"));
        assert!(body.contains("target:            local:/run/pmacs/work.sock"));
    }
}
