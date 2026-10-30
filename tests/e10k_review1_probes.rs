//! E10k review 1 --- behavioral probes against E10k's rows, in states a
//! user reaches that the phase's witnesses did not choose, committed as
//! witnesses at fix round 1.
//!
//! P1 (High 1): E10k.3 and E10k.4 hold a replica's and the daemon's
//! per-key reads to a walk of loro's tree and a scan of one element, and
//! `def258d` bounds an element by writing large text in pieces. The
//! arbiter's undo wrote through loro's own `undo_internal`, which applies
//! the restored text as one insert, one element of whatever size: after
//! `C-x h`, `C-w`, `C-x u` on #414's 18 MiB log, every byte lookup at the
//! end scanned the whole text again (0.150 against 54.2 ms a key's reads
//! in release at `700e86f`, 362 times). Since fix round 1 an undo writes
//! the text it restores in pieces. P1 is a ratio of
//! wall-clock times, so a budget (D12): `#[ignore]`, run by exact name by
//! `scripts/perf-budgets`. Its structural form, which no load can fail,
//! is `crdt::tests::e10k_an_undone_cut_and_a_redone_paste_come_back_in_pieces`
//! in the default sweep.
//!
//! P5 (Low 4): dabbrev dropped every word touching an edge its window
//! drew, so a whole word beginning at the window's first byte, a space
//! before it, or ending at its last, a space after it, was not offered,
//! though E10k.2's coherence statement ("none beyond the window") and the
//! provider's own comment ("nor one the window's edge cuts") say it is.
//! Since fix round 1 the byte beyond each drawn edge is read, and counted,
//! and a word is dropped only when that byte is a word byte.
//!
//! The review's P2a and P2b (Low 2) are committed in
//! `tests/e10k_replica_reads_acceptance.rs`, whose private guard they
//! plant against; P3 (High 1, through the attached TUI and the GPU) as
//! two rows of `tests/e10k_large_buffer_typing.rs`.

#[cfg(feature = "crdt")]
use std::time::{Duration, Instant};

#[cfg(feature = "crdt")]
use pmacs::crdt::{ChangeBase, CrdtState};

/// E10k's fixture: `lines` log lines of 70 bytes, the last ending `zq`.
#[cfg(feature = "crdt")]
fn fixture(lines: usize) -> String {
    let mut out = String::with_capacity(lines * 70);
    for i in 0..lines {
        let mut line = format!(
            "2026-10-27T12:{:02}:{:02}.{:03}Z INFO daemon: served request {i:07} in {:03}us",
            (i / 60_000) % 60,
            (i / 1000) % 60,
            i % 1000,
            i % 997
        );
        assert_eq!(line.len(), 69);
        if i + 1 == lines {
            line.replace_range(66.., " zq");
        }
        line.push('\n');
        out.push_str(&line);
    }
    out
}

/// What one typed key at the end asks of a state: the mirror's
/// `char_before_byte` (twice a helper) and the daemon's conversion of an
/// imported insert's position (`unicode_to_utf8_pos`), `rounds` times.
#[cfg(feature = "crdt")]
fn per_key_reads(s: &CrdtState, rounds: usize) -> Duration {
    let len = s.len_utf8();
    let chars = s.len_unicode();
    let t0 = Instant::now();
    for _ in 0..rounds {
        assert_eq!(std::hint::black_box(s.char_before_byte(len)), Some('\n'));
        assert_eq!(
            std::hint::black_box(s.unicode_to_utf8_pos(chars - 1)),
            Some(len - 1)
        );
    }
    t0.elapsed() / u32::try_from(rounds).expect("rounds")
}

/// P1: the whole buffer deleted and the deletion undone through the
/// arbiter's primitive keeps its text, and keeps the per-key reads as
/// cheap as the pieces made them. At `700e86f` the undone text was one
/// element and the reads cost what a whole-text scan costs.
#[test]
#[ignore = "wall-clock budget (D12): run by scripts/perf-budgets and CI's perf jobs"]
fn p1_an_undone_whole_buffer_delete_keeps_the_per_key_reads_bounded() {
    #[cfg(feature = "crdt")]
    {
        let text = fixture(269_634);
        let s = CrdtState::from_bytes(1, text.as_bytes()).expect("seed");
        let rounds = 5;
        let pieces = per_key_reads(&s, rounds);

        let before = s.version();
        let ((), _) = s.capture(|| s.delete(0, s.len_utf8()).expect("delete all"));
        let span = s
            .spans_since(&before)
            .into_iter()
            .next()
            .expect("the delete's span");
        assert_eq!(s.len_utf8(), 0);
        s.undo_span_against(span, &mut ChangeBase::default())
            .expect("undo");
        assert_eq!(s.len_utf8(), text.len(), "the undo restored the text");
        assert!(s.materialize_string() == text, "the same text");

        let undone = per_key_reads(&s, rounds);
        eprintln!(
            "p1: per key, in pieces {pieces:?}; after the delete was undone {undone:?} ({:.0}x)",
            undone.as_secs_f64() / pieces.as_secs_f64()
        );
        assert!(
            undone < pieces * 10 + Duration::from_millis(1),
            "after C-x h C-w C-x u a key's reads cost {undone:?} against {pieces:?} in pieces: \
             the undo wrote the text back as one element"
        );
    }
    #[cfg(not(feature = "crdt"))]
    eprintln!("skipping: the replicas' reads are a crdt build's");
}

/// P5: dabbrev offers no word "the window's edge cuts" (its comment,
/// `src/completion_framework.rs`), and skipped every word that touched an
/// edge the window drew, so a whole word that begins exactly at the
/// window's first byte, a space before it, was dropped though nothing cut
/// it; the same at the window's last byte. E10k.2's coherence statement
/// is "dabbrev offers words near the cursor first and none beyond the
/// window": these words are inside it.
#[test]
fn p5_a_whole_word_at_the_window_edge_is_offered() {
    use pmacs::completion_framework::{CompletionContext, dabbrev_provider};
    // Before the cursor: `zqedge` starts at byte 11, a space at 10; the
    // cursor at the end, the window reaching back exactly to byte 11.
    let text = format!("{} zqedge{}zq", "y".repeat(10), " ".repeat(20));
    let cursor = text.len();
    let window = cursor - 11;
    assert_eq!(&text[11..17], "zqedge", "fixture");
    let f = dabbrev_provider(move |_| window);
    let mut c = CompletionContext::new("zq", text.as_str());
    c.cursor = Some(cursor);
    let before: Vec<String> = f(&c).into_iter().map(|i| i.label).collect();
    // After the cursor: `zqtail` ends at the window's last byte, a space
    // after it.
    let text = format!("zq {}zqtail {}", " ".repeat(20), "y".repeat(10));
    let end = text.find("zqtail").expect("fixture") + "zqtail".len();
    let f = dabbrev_provider(move |_| end - 2);
    let mut c = CompletionContext::new("zq", text.as_str());
    c.cursor = Some(2);
    let after: Vec<String> = f(&c).into_iter().map(|i| i.label).collect();
    assert_eq!(
        (before, after),
        (vec!["zqedge".to_owned()], vec!["zqtail".to_owned()]),
        "a word wholly inside the window, touching its edge, is offered"
    );
}
