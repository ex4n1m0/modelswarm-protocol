const IMPLEMENTED: Array<{ method: string; path: string; note: string }> = [
  { method: "GET", path: "/api/v1/health", note: "liveness + protocol version" },
];

const PLANNED_PHASE_1: Array<{ method: string; path: string }> = [
  { method: "GET", path: "/api/v1/catalog" },
  { method: "GET", path: "/api/v1/catalog/{profile_id}" },
  { method: "POST", path: "/api/v1/auth/device/start" },
  { method: "POST", path: "/api/v1/auth/device/complete" },
  { method: "POST", path: "/api/v1/peers/register" },
  { method: "POST", path: "/api/v1/peers/heartbeat" },
  { method: "POST", path: "/api/v1/peers/challenge/complete" },
  { method: "GET", path: "/api/v1/peers?profile_id=…" },
  { method: "POST", path: "/api/v1/rendezvous/offer" },
  { method: "POST", path: "/api/v1/rendezvous/answer" },
  { method: "POST", path: "/api/v1/events/job-result" },
  { method: "POST", path: "/api/v1/peers/drain" },
];

export default function Home() {
  return (
    <main style={{ fontFamily: "monospace", maxWidth: 720, margin: "3rem auto", padding: "0 1rem" }}>
      <h1>ModelSwarm Hub</h1>
      <p>
        Control plane only: catalog, rendezvous, leases, policy. It never
        carries prompts, completions, model files, or inference traffic
        (ADR-001).
      </p>
      <h2>Implemented</h2>
      <ul>
        {IMPLEMENTED.map((e) => (
          <li key={e.path}>
            <code>{e.method} {e.path}</code> — {e.note}
          </li>
        ))}
      </ul>
      <h2>Planned in Phase 1</h2>
      <ul>
        {PLANNED_PHASE_1.map((e) => (
          <li key={e.path}>
            <code>{e.method} {e.path}</code>
          </li>
        ))}
      </ul>
    </main>
  );
}
