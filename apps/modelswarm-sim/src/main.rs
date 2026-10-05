//! Thin CLI for the scenario runner. All logic lives in `modelswarm_sim`
//! (lib) so tests exercise scenarios without spawning processes.
//!
//! Usage:
//!
//! ```text
//! modelswarm-sim pair
//! modelswarm-sim mesh <n>
//! modelswarm-sim kill <at_ms>
//! ```
//!
//! Each run prints exactly one JSON object on stdout and exits. Exit code 0
//! means the scenario itself ran (including `kill`, whose JSON documents the
//! observed peer failure); exit code 2 is a usage error; exit code 1 means
//! the scenario errored before producing its event (also as one JSON line).

use std::process::ExitCode;

use serde_json::json;

const USAGE: &str = "usage: modelswarm-sim <scenario>\n  scenarios: pair | mesh <n> | kill <at_ms>";
const DEFAULT_RTT_SAMPLES: usize = 8;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let scenario = match args.as_slice() {
        [_bin, cmd, rest @ ..] => (cmd.as_str(), rest),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(scenario).await {
        Ok(event) => {
            println!("{event}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            let event = json!({
                "ok": false,
                "event": "scenario_error",
                "error": format!("{err:#}"),
            });
            println!("{event}");
            ExitCode::FAILURE
        }
    }
}

async fn run(scenario: (&str, &[String])) -> anyhow::Result<serde_json::Value> {
    match scenario {
        ("pair", []) => modelswarm_sim::run_pair(DEFAULT_RTT_SAMPLES).await,
        ("mesh", [n]) => {
            let n: usize = n
                .parse()
                .map_err(|_| anyhow::anyhow!("mesh: <n> must be a peer count in 2..=16"))?;
            modelswarm_sim::run_mesh(n, DEFAULT_RTT_SAMPLES).await
        }
        ("kill", [at_ms]) => {
            let at_ms: u64 = at_ms.parse().map_err(|_| {
                anyhow::anyhow!("kill: <at_ms> must be a millisecond deadline >= 0")
            })?;
            modelswarm_sim::run_kill(at_ms).await
        }
        (cmd, _) => anyhow::bail!("unknown scenario or arguments: {cmd}\n{USAGE}"),
    }
}
