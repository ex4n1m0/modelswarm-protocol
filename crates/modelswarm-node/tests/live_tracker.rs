//! Live-network probe (ignored by default; run with -- --ignored):
//! verifies the BUILT CLIENT's HTTPS path against the production tracker:
//! health, verified catalog fetch, and active-profile parsing.

use modelswarm_identity::InstallationIdentity;
use modelswarm_tracker_api::TrackerClient;
use std::sync::Arc;

#[tokio::test]
#[ignore = "hits production modelswarm.deepflux.space"]
async fn production_catalog_verifies_over_https() {
    let identity = Arc::new(InstallationIdentity::from_bytes(&[7u8; 32]));
    let tracker = TrackerClient::new("https://modelswarm.deepflux.space", identity);
    let health = tracker.health().await.expect("health over https");
    assert_eq!(health["status"], "ok");

    let catalog = modelswarm_node::catalog::active_profiles(&tracker)
        .await
        .expect("verified catalog");
    assert!(!catalog.is_empty(), "at least one active profile");
    for p in &catalog {
        println!("{} -> {}", p.display_name, p.profile_id);
    }
}
