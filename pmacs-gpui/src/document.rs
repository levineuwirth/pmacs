use std::sync::{Arc, Mutex};

use loro::{ContainerTrait, LoroDoc, TextDelta};
use ropey::Rope;

/// Loro owns the replica. The line index lets GPUI ask for just the visible rows.
pub struct Document {
    replica: LoroDoc,
    lines: Arc<Mutex<Rope>>,
    _subscription: loro::Subscription,
}

impl Document {
    pub fn from_snapshot(snapshot: &[u8]) -> Result<Self, loro::LoroError> {
        let replica = LoroDoc::new();
        replica.import(snapshot)?;
        let body = replica.get_text("body");
        let lines = Arc::new(Mutex::new(Rope::from_str(&body.to_string())));
        let changed = Arc::clone(&lines);
        let subscription = replica.subscribe(
            &body.id(),
            Arc::new(move |batch| {
                let mut lines = changed.lock().expect("document line index");
                for event in batch.events {
                    if let Some(delta) = event.diff.as_text() {
                        let mut cursor = 0;
                        for change in delta {
                            match change {
                                TextDelta::Retain { retain, .. } => cursor += retain,
                                TextDelta::Insert { insert, .. } => {
                                    lines.insert(cursor, insert);
                                    cursor += insert.chars().count();
                                }
                                TextDelta::Delete { delete } => {
                                    lines.remove(cursor..cursor + delete)
                                }
                            }
                        }
                    }
                }
            }),
        );
        Ok(Self {
            replica,
            lines,
            _subscription: subscription,
        })
    }

    pub fn import(&self, bytes: &[u8]) -> Result<(), loro::LoroError> {
        self.replica.import(bytes)?;
        Ok(())
    }

    pub fn with_lines<T>(&self, read: impl FnOnce(&Rope) -> T) -> T {
        read(&self.lines.lock().expect("document line index"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loro::ExportMode;

    #[test]
    fn unicode_edits_keep_visible_lines_in_sync_across_replayed_updates() {
        let author = LoroDoc::new();
        let body = author.get_text("body");
        body.insert(0, "a😀b\nlast\n").unwrap();
        let snapshot = author.export(ExportMode::Snapshot).unwrap();
        let view = Document::from_snapshot(&snapshot).unwrap();
        let version = author.oplog_vv();
        body.delete(1, 2).unwrap();
        body.insert(1, "中\nnew").unwrap();
        let update = author.export(ExportMode::updates(&version)).unwrap();
        for _ in 0..2 {
            view.import(&update).unwrap();
            view.with_lines(|lines| {
                assert_eq!(lines.len_lines(), 4);
                assert_eq!(lines.to_string(), "a中\nnew\nlast\n");
            });
        }
    }
}
