// tests/e10c_review1_measure.rs --- E10c review 1's measurement of naming.

//! A measurement, not a witness (Medium 1): N files opened in one fresh
//! session by `pmacs.buffer.find_or_open` in one Lua loop, timed around
//! it, then one kill timed; unique basenames (`f<i>.txt`) and all
//! colliding ones (`d<i>/mod.txt`), no grammar and no language server.
//! `#[ignore]`d and asserting nothing; run by hand with `--ignored
//! --nocapture` and read the `R1MEASURE` lines.

use std::time::Instant;

use pmacs::editor::EditorState;

#[path = "common/editor.rs"]
mod editor;
use editor::exec;

fn fresh() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    exec(&s, "pmacs.lsp.config = {}");
    s
}

#[test]
#[ignore = "a measurement, run by hand"]
fn r1_measure_opening_n_files() {
    for &(n, colliding) in &[
        (250usize, false),
        (500, false),
        (1000, false),
        (2000, false),
        (250, true),
        (500, true),
        (1000, true),
        (2000, true),
    ] {
        let td = tempfile::tempdir().expect("tempdir");
        let mut paths = Vec::with_capacity(n);
        for i in 0..n {
            let p = if colliding {
                td.path().join(format!("d{i}/mod.txt"))
            } else {
                td.path().join(format!("f{i}.txt"))
            };
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x\n").unwrap();
            paths.push(p.display().to_string());
        }
        let s = fresh();
        let list = paths
            .iter()
            .map(|p| format!("{p:?}"))
            .collect::<Vec<_>>()
            .join(",");
        exec(&s, &format!("_G.R1_PATHS = {{ {list} }}"));
        let t0 = Instant::now();
        exec(
            &s,
            "for _, p in ipairs(_G.R1_PATHS) do pmacs.buffer.find_or_open(p) end",
        );
        let open = t0.elapsed();
        let t1 = Instant::now();
        exec(
            &s,
            "local ids = pmacs.buffer.list(); pmacs.buffer.kill(ids[#ids])",
        );
        let kill = t1.elapsed();
        eprintln!(
            "R1MEASURE n={n} colliding={colliding} open_total_ms={} per_open_us={} last_kill_us={}",
            open.as_millis(),
            open.as_micros() / n as u128,
            kill.as_micros()
        );
    }
}

#[path = "common/iso.rs"]
mod iso;
