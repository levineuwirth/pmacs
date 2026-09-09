// tests/common/hello_measure.rs --- MEASUREMENT INSTRUMENT, not for merge.
//
// The `read Hello` family (#258) is a fixture reading the daemon's
// server-first `Hello` under a short read timeout it sets itself. Every
// record so far argued about that interval from the CI logs; nothing
// measured it on the platform that fails. This module times the read
// at the sites that have failed and at the one readiness wait, writes
// each sample to a file the last-sorted suite reports, and keeps each
// fixture's own budget as the pass/fail criterion so a red says how
// long `Hello` actually took instead of `WouldBlock`.

#![allow(dead_code)]

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pmacs::protocol::Hello;
use pmacs::transport::read_message;

/// The cap on how long a timed read waits before it is recorded as a
/// timeout. Generous so the tail is measured rather than truncated.
pub const CAP: Duration = Duration::from_secs(20);

/// Where samples accumulate: the checkout root, which every test
/// binary of the root package sees as `CARGO_MANIFEST_DIR`.
pub fn sample_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("hello-measure.tsv")
}

/// Append one sample: site, the fixture's budget in ms, the observed
/// interval in ms, and the outcome (`ok`, `timeout`, or an error kind).
pub fn record(site: &str, budget: Duration, elapsed: Duration, outcome: &str) {
    let line = format!(
        "HELLO-MEASURE\t{site}\t{}\t{:.2}\t{outcome}\n",
        budget.as_millis(),
        elapsed.as_secs_f64() * 1000.0
    );
    eprint!("{line}");
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(sample_path())
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Read `Hello` on a freshly connected stream, timing it against the
/// fixture's own `budget`. The read waits up to [`CAP`]; the sample is
/// recorded either way; then the stream's read timeout is restored to
/// `budget` so the rest of the fixture runs exactly as before, and the
/// fixture's criterion is enforced with the elapsed time in the message.
pub fn hello_within(stream: &mut UnixStream, site: &str, budget: Duration) -> Hello {
    stream.set_read_timeout(Some(CAP)).unwrap();
    let start = Instant::now();
    let result = read_message::<Hello>(stream);
    let elapsed = start.elapsed();
    stream.set_read_timeout(Some(budget)).unwrap();
    match result {
        Ok(hello) => {
            record(site, budget, elapsed, "ok");
            assert!(
                elapsed <= budget,
                "{site}: Hello took {:.2} ms against a {} ms budget",
                elapsed.as_secs_f64() * 1000.0,
                budget.as_millis()
            );
            hello
        }
        Err(error) => {
            record(site, budget, elapsed, &format!("{error:?}"));
            panic!(
                "{site}: no Hello within {:.2} ms (cap {} ms, budget {} ms): {error:?}",
                elapsed.as_secs_f64() * 1000.0,
                CAP.as_millis(),
                budget.as_millis()
            );
        }
    }
}
