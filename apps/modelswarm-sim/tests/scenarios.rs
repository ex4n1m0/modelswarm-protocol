//! Sim scenarios invoked as library functions (no process spawning): the
//! same code paths the CLI runs, asserted on their JSON summaries.

use modelswarm_sim::{run_kill, run_mesh, run_pair};

#[tokio::test]
async fn pair_produces_a_complete_summary() {
    let summary = run_pair(5).await.expect("pair scenario");
    assert_eq!(summary["scenario"], "pair");
    assert_eq!(summary["ok"], true);
    assert_eq!(summary["text"], "Hello");
    assert_eq!(summary["token_deltas"], 2);
    assert_eq!(summary["finish_reason"], "stop");
    assert_eq!(summary["prompt_tokens"], 12);
    let p50 = summary["rtt_p50_ms"].as_f64().expect("p50 numeric");
    assert!((0.0..250.0).contains(&p50), "loopback rtt insane: {p50}");
}

#[tokio::test]
async fn mesh_forms_a_complete_adjacency_with_rtt_matrix() {
    let n = 4;
    let summary = run_mesh(n, 2).await.expect("mesh scenario");
    assert_eq!(summary["scenario"], "mesh");
    assert_eq!(summary["ok"], true);
    assert_eq!(summary["peers"], n);
    assert_eq!(
        summary["peer_ids"].as_array().map(Vec::len),
        Some(n),
        "one peer id per peer"
    );
    let adjacency = summary["adjacency"].as_array().expect("adjacency matrix");
    assert_eq!(adjacency.len(), n);
    for (i, row) in adjacency.iter().enumerate() {
        let row = row.as_array().expect("adjacency row");
        assert_eq!(row.len(), n);
        for (j, cell) in row.iter().enumerate() {
            let expected = if i == j { 0 } else { 1 };
            assert_eq!(cell.as_u64(), Some(expected), "adjacency[{i}][{j}]");
        }
    }
    let rtt = summary["rtt_p50_ms"].as_array().expect("rtt matrix");
    for (i, row) in rtt.iter().enumerate() {
        for (j, cell) in row.as_array().unwrap().iter().enumerate() {
            let v = cell.as_f64().expect("rtt numeric");
            assert!(v.is_finite() && v >= 0.0, "rtt[{i}][{j}] = {v}");
            if i == j {
                assert_eq!(v, 0.0);
            } else if i < j {
                assert!(v < 250.0, "loopback rtt[{i}][{j}] insane: {v}");
            }
        }
    }
}

#[tokio::test]
async fn kill_reports_an_explicit_failure_without_hanging() {
    let started = std::time::Instant::now();
    let event = run_kill(150)
        .await
        .expect("kill scenario runs to its event");
    let elapsed = started.elapsed();
    assert_eq!(event["scenario"], "kill");
    assert_eq!(event["ok"], false);
    assert_eq!(event["event"], "peer_failure");
    assert_eq!(event["hang"], false);
    let observed = event["observed"].as_str().expect("observed string");
    assert!(
        observed == "closed" || observed == "timeout",
        "expected closed or timeout, got {observed}"
    );
    assert!(
        event["msp_code"].as_str().is_some(),
        "failure carries an msp-v1 §6.5 code"
    );
    assert!(event["token_deltas_received"].as_u64() >= Some(1));
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "scenario must be bounded, took {elapsed:?}"
    );
}
