const IMPLEMENTED: Array<{ method: string; path: string; note: string }> = [
  { method: "GET", path: "/api/v1/health", note: "liveness + protocol version" },
  { method: "GET", path: "/api/v1/catalog", note: "signed catalog envelope (schema v2)" },
  { method: "GET", path: "/api/v1/catalog/{profile_id}", note: "single-profile envelope" },
  { method: "POST", path: "/api/v1/auth/device/start", note: "device enrollment start" },
  { method: "POST", path: "/api/v1/auth/device/complete", note: "session issuance (403 pending until approved)" },
  { method: "POST", path: "/api/v1/peers/register", note: "lease issuance (60–90 s TTL)" },
  { method: "POST", path: "/api/v1/peers/heartbeat", note: "lease extension + revocation notices" },
  { method: "POST", path: "/api/v1/peers/drain", note: "exclude peer from lookups" },
  { method: "POST", path: "/api/v1/peers/challenge/start", note: "hosting challenge" },
  { method: "POST", path: "/api/v1/peers/challenge/complete", note: "record timings, mark passed" },
  { method: "POST", path: "/api/v1/peers/lease", note: "eligibility lease issuance (ADR-012)" },
  { method: "GET", path: "/api/v1/peers?profile_id=…", note: "peer lookup, limit ≤ 50" },
  { method: "POST", path: "/api/v1/rendezvous/offer", note: "signaling relay" },
  { method: "POST", path: "/api/v1/rendezvous/answer", note: "signaling relay" },
  { method: "GET", path: "/api/v1/rendezvous/pending", note: "mailbox drain (delete-on-read)" },
  { method: "POST", path: "/api/v1/events/job-result", note: "receipt digest + outcome" },
  { method: "POST", path: "/api/v1/receipt", note: "canonical receipt intake (Phase B)" },
  { method: "POST", path: "/api/v1/session-authorize", note: "cooperative session metadata (Phase B)" },
  { method: "POST", path: "/api/v1/audit", note: "audit-epoch sweep (admin, Phase B)" },
  { method: "POST", path: "/api/v1/admin/catalog/candidates", note: "candidate insert (admin)" },
  { method: "POST", path: "/api/v1/admin/catalog/promote", note: "candidate → active (admin)" },
];

export default function Home() {
  return (
    <main style={{ fontFamily: "monospace", maxWidth: 720, margin: "3rem auto", padding: "0 1rem" }}>
      <h1>ModelSwarm Tracker</h1>
      <p>
        Control plane only: catalog, rendezvous, leases, policy. It never
        carries prompts, completions, model files, or inference traffic
        (ADR-001).
      </p>
      <h2>Implemented (Phase B)</h2>
      <ul>
        {IMPLEMENTED.map((e) => (
          <li key={e.path}>
            <code>{e.method} {e.path}</code> — {e.note}
          </li>
        ))}
      </ul>
    </main>
  );
}
