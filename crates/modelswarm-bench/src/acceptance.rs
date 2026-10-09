//! Per-(peer, profile) speculative-acceptance EWMAs for the 9.6 pass-1
//! harness (audit gap §6.4: "No acceptance-EWMA store — the 0.8 constant
//! is the stand-in"; expanded-mission §5: "speculative replaces the
//! synthetic 0.8 acceptance constant with measured acceptance EWMA").
//!
//! Every speculative round records `(accepted_tokens, proposed_tokens)`
//! against the PROPOSER peer + profile; the EWMA (per-observation alpha =
//! the frozen ADR-013 [`modelswarm_scheduler::EWMA_ALPHA`]) plus the
//! cumulative counters persist through the node-local store's additive
//! migration 4 (`speculative_acceptance_observations`), so the planner
//! arm resumes measured acceptance across restarts instead of resetting
//! to the TEST-ONLY prior.
//!
//! Honesty: this is EXPERIMENT data (harness-owned). Nothing in the
//! production serving/chat path reads or writes it. Counts only — never
//! token text (the store's structural privacy audit covers the table).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use modelswarm_scheduler::EWMA_ALPHA;

/// One (peer, profile) acceptance state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcceptanceState {
    /// EWMA of per-round acceptance (accepted/proposed).
    pub ewma: f64,
    /// Rounds recorded (lifetime, persisted).
    pub rounds: u64,
    /// Accepted tokens (lifetime, persisted).
    pub accepted_count: u64,
    /// Proposed tokens (lifetime, persisted).
    pub proposed_count: u64,
}

impl AcceptanceState {
    /// The lifetime aggregate acceptance (accepted/proposed counts).
    #[must_use]
    pub fn lifetime_rate(&self) -> f64 {
        if self.proposed_count == 0 {
            0.0
        } else {
            self.accepted_count as f64 / self.proposed_count as f64
        }
    }

    fn update(&mut self, accepted: u32, proposed: u32) {
        let round_rate = f64::from(accepted) / f64::from(proposed.max(1));
        // First observation seeds the EWMA (the F15 `ewma(None)` pattern)
        // so a cold pair is not biased toward 0.
        self.ewma = if self.rounds == 0 {
            round_rate
        } else {
            EWMA_ALPHA * round_rate + (1.0 - EWMA_ALPHA) * self.ewma
        };
        self.rounds += 1;
        self.accepted_count += u64::from(accepted);
        self.proposed_count += u64::from(proposed);
    }
}

/// Persistence failures (best-effort contract: a store error degrades to
/// memory-only with the error recorded; it never takes a round down).
pub type AcceptanceError = String;

/// The acceptance-EWMA store: in-memory state + best-effort SQLite
/// persistence (migration 4). Clone-free by design — one instance per
/// harness process, shared by reference.
pub struct AcceptanceStore {
    state: Mutex<BTreeMap<(String, String), AcceptanceState>>,
    store: Option<modelswarm_store::Store>,
    /// Set when a persistence error occurred (surfaced in artifacts).
    pub persist_error: Mutex<Option<AcceptanceError>>,
}

impl AcceptanceStore {
    /// Memory-only (fast loopback tests that do not need resume).
    #[must_use]
    pub fn memory() -> Self {
        Self {
            state: Mutex::new(BTreeMap::new()),
            store: None,
            persist_error: Mutex::new(None),
        }
    }

    /// Persistent (file-backed; the resume path the planner arm uses).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AcceptanceError> {
        let store = modelswarm_store::Store::open(path)
            .map_err(|e| format!("open acceptance store: {e}"))?;
        let mut state = BTreeMap::new();
        // Hydrate: every persisted row becomes live state.
        for ((peer, profile), (ewma, rounds, accepted, proposed)) in store.all_acceptance_rows() {
            state.insert(
                (peer, profile),
                AcceptanceState {
                    ewma,
                    rounds,
                    accepted_count: accepted,
                    proposed_count: proposed,
                },
            );
        }
        Ok(Self {
            state: Mutex::new(state),
            store: Some(store),
            persist_error: Mutex::new(None),
        })
    }

    /// Records one round's acceptance against (proposer peer, profile).
    /// Updates the EWMA + counters in memory and persists best-effort.
    pub fn record_round(&self, peer_id: &str, profile_id: &str, accepted: u32, proposed: u32) {
        let key = (peer_id.to_string(), profile_id.to_string());
        let snapshot = {
            let mut state = self.state.lock().expect("acceptance state lock");
            let entry = state.entry(key).or_insert(AcceptanceState {
                ewma: 0.0,
                rounds: 0,
                accepted_count: 0,
                proposed_count: 0,
            });
            entry.update(accepted, proposed);
            *entry
        };
        if let Some(store) = &self.store {
            let persisted = store.upsert_acceptance(
                peer_id,
                profile_id,
                snapshot.ewma,
                snapshot.rounds,
                snapshot.accepted_count,
                snapshot.proposed_count,
            );
            if let Err(e) = persisted {
                *self.persist_error.lock().expect("persist error lock") =
                    Some(format!("upsert acceptance: {e}"));
            }
        }
    }

    /// The measured acceptance EWMA for (peer, profile), when any round
    /// has been recorded against it.
    #[must_use]
    pub fn acceptance_ewma(&self, peer_id: &str, profile_id: &str) -> Option<f64> {
        let state = self.state.lock().expect("acceptance state lock");
        state
            .get(&(peer_id.to_string(), profile_id.to_string()))
            .map(|s| s.ewma)
    }

    /// A serializable snapshot of every (peer, profile) state (ids,
    /// counts, and rates only).
    #[must_use]
    pub fn snapshot(&self) -> Vec<(String, String, AcceptanceState)> {
        let state = self.state.lock().expect("acceptance state lock");
        state
            .iter()
            .map(|((peer, profile), s)| (peer.clone(), profile.clone(), *s))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn ewma_converges_and_counters_accumulate() {
        let store = AcceptanceStore::memory();
        // Ten perfect rounds then ten zero rounds: the EWMA must fall
        // from 1.0 toward 0 without ever leaving [0, 1].
        for _ in 0..10 {
            store.record_round("p", "msp1:a", 8, 8);
        }
        assert!(approx(store.acceptance_ewma("p", "msp1:a").unwrap(), 1.0));
        for _ in 0..10 {
            store.record_round("p", "msp1:a", 0, 8);
        }
        let ewma = store.acceptance_ewma("p", "msp1:a").unwrap();
        assert!(ewma < 0.05, "converged downward: {ewma}");
        let snap = &store.snapshot()[0];
        assert_eq!(snap.2.rounds, 20);
        assert_eq!(snap.2.accepted_count, 80);
        assert_eq!(snap.2.proposed_count, 160);
        assert!(approx(snap.2.lifetime_rate(), 0.5));
    }

    #[test]
    fn keys_are_peer_and_profile_scoped() {
        let store = AcceptanceStore::memory();
        store.record_round("p1", "msp1:a", 8, 8);
        store.record_round("p1", "msp1:b", 0, 8);
        store.record_round("p2", "msp1:a", 4, 8);
        assert!(approx(store.acceptance_ewma("p1", "msp1:a").unwrap(), 1.0));
        assert!(approx(store.acceptance_ewma("p1", "msp1:b").unwrap(), 0.0));
        assert!(approx(store.acceptance_ewma("p2", "msp1:a").unwrap(), 0.5));
        assert_eq!(store.acceptance_ewma("p3", "msp1:a"), None);
        assert_eq!(store.snapshot().len(), 3);
    }

    #[test]
    fn persistence_resumes_counters_and_ewma() {
        let dir = tempfile::NamedTempFile::new().unwrap();
        let path = dir.path().to_path_buf();
        {
            let store = AcceptanceStore::open(&path).expect("open");
            for _ in 0..4 {
                store.record_round("p", "msp1:a", 8, 8);
            }
            for _ in 0..4 {
                store.record_round("p", "msp1:a", 0, 8);
            }
            assert!(store.persist_error.lock().unwrap().is_none());
            let ewma = store.acceptance_ewma("p", "msp1:a").unwrap();
            assert!(ewma < 1.0 && ewma > 0.0, "mixed rounds: {ewma}");
        }
        let resumed = AcceptanceStore::open(&path).expect("reopen");
        let state = &resumed.snapshot()[0].2;
        assert_eq!(state.rounds, 8, "lifetime counters resume");
        assert_eq!(state.accepted_count, 32);
        assert_eq!(state.proposed_count, 64);
        // The persisted EWMA itself resumes (not recomputed from counts).
        assert!(approx(
            resumed.acceptance_ewma("p", "msp1:a").unwrap(),
            state.ewma
        ));
    }
}
