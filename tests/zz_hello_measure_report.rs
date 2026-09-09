// tests/zz_hello_measure_report.rs --- MEASUREMENT INSTRUMENT, not for merge.
//
// Sorted last among the integration suites so it runs after every
// daemon-spawning fixture in the job. It reads the samples the
// instrument appended and fails deliberately, because a failing test
// is the only output `cargo test` prints without `--nocapture`.

//! The `Hello` measurement collector (#258); not for merge.

mod common;

use std::collections::BTreeMap;

#[test]
fn zz_hello_measure_report() {
    let path = common::hello_measure::sample_path();
    let body = std::fs::read_to_string(&path).unwrap_or_default();
    let mut by_site: BTreeMap<String, (u128, Vec<f64>, usize, usize)> = BTreeMap::new();
    for line in body.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 5 || fields[0] != "HELLO-MEASURE" {
            continue;
        }
        let budget: u128 = fields[2].parse().unwrap_or(0);
        let elapsed: f64 = fields[3].parse().unwrap_or(0.0);
        let entry = by_site
            .entry(fields[1].to_owned())
            .or_insert((budget, Vec::new(), 0, 0));
        entry.1.push(elapsed);
        if elapsed > budget as f64 {
            entry.2 += 1;
        }
        if fields[4] != "ok" {
            entry.3 += 1;
        }
    }
    let mut report = String::new();
    report.push_str("site\tbudget_ms\tn\tp50_ms\tp90_ms\tmax_ms\tover_budget\tnot_ok\n");
    for (site, (budget, mut samples, over, not_ok)) in by_site {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = samples.len();
        let at = |q: f64| samples[((n as f64 - 1.0) * q).round() as usize];
        report.push_str(&format!(
            "{site}\t{budget}\t{n}\t{:.2}\t{:.2}\t{:.2}\t{over}\t{not_ok}\n",
            at(0.5),
            at(0.9),
            samples[n - 1]
        ));
    }
    panic!(
        "HELLO-MEASURE REPORT ({} samples from {})\n{report}\n--- raw ---\n{body}",
        body.lines().count(),
        path.display()
    );
}
