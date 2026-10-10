//! Joins `sched.shadow.decision` (shadow plan) with
//! `sched.shadow.realized` (production outcome) out of a node/desktop
//! telemetry JSONL file — the 9.6 divergence dataset builder.
//!
//! The join is a LEFT join on the realized side: every realized row is
//! kept (including `served_by=failed` pre-plan rows that have NO decision
//! row — those carry `has_plan: false` with null plan fields); decision
//! rows without a realized outcome are counted and dropped visibly.
//!
//! Usage:
//! ```text
//! cargo run -p modelswarm-bench --example shadow_join -- \
//!     <telemetry.jsonl> <shadow-join.jsonl>
//! ```
//! Stats print to stderr; the joined rows land in the output file.

use std::io::BufRead;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: shadow_join <telemetry.jsonl> <shadow-join.jsonl>");
        std::process::exit(2);
    };
    let file = std::fs::File::open(&input).unwrap_or_else(|e| {
        eprintln!("open {input}: {e}");
        std::process::exit(1);
    });
    let lines: Vec<String> = std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .collect();
    let report = modelswarm_bench::shadow_join::join_shadow_telemetry(lines);
    let stats = &report.stats;
    eprintln!(
        "shadow join: {realized} realized rows -> {rows} emitted ({joined} with plan, \
         {only} realized-only LEFT-kept, {dropped} decision-only dropped, \
         {unrelated} unrelated, {unparsable} unparsable)",
        realized = stats.realized_rows,
        rows = report.rows.len(),
        joined = stats.joined_with_plan,
        only = stats.realized_only,
        dropped = stats.decision_only_dropped,
        unrelated = stats.unrelated_lines,
        unparsable = stats.unparsable_lines,
    );
    let mut out = String::new();
    for row in &report.rows {
        out.push_str(&serde_json::to_string(row).unwrap_or_else(|e| {
            eprintln!("serialize row: {e}");
            std::process::exit(1);
        }));
        out.push('\n');
    }
    std::fs::write(&output, out).unwrap_or_else(|e| {
        eprintln!("write {output}: {e}");
        std::process::exit(1);
    });
}
