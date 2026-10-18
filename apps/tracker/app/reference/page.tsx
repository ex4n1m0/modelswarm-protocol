// Reference page (owner directive 2026-10-17: presets + the tracker API
// table move OFF the main page, behind the top menu). Same copy rules as
// the main page: honest tense only, no provider/runtime implementation
// words in app/ source (the H2 meta contract), no unimplemented claims.

// Protocol-level execution presets (ADR-029 §1; master prompt §3).
const PRESETS: Array<{ name: string; allows: string; correctness: string }> = [
  { name: "FAST", allows: "single host", correctness: "lossless — the baseline" },
  { name: "BALANCED", allows: "single + hedged (≤3 raced peers)", correctness: "lossless" },
  { name: "DEEP", allows: "best-of-n + deliberation roles", correctness: "approximate — never lossless" },
  { name: "VERIFIED", allows: "BALANCED + audited chain", correctness: "carries audit proofs" },
  { name: "MAXIMUM", allows: "sanctioned composition of gated modes", correctness: "union of component labels" },
];

const ENDPOINTS: Array<{ method: string; path: string; note: string }> = [
  { method: "GET", path: "/api/v1/health", note: "liveness + protocol version" },
  { method: "GET", path: "/api/v1/catalog", note: "signed catalog envelope (schema v2)" },
  { method: "GET", path: "/api/v1/catalog/{profile_id}", note: "single-profile envelope" },
  { method: "POST", path: "/api/v1/auth/device/start", note: "device enrollment start" },
  { method: "POST", path: "/api/v1/auth/device/complete", note: "session issuance (403 pending until approved)" },
  { method: "POST", path: "/api/v1/admin/devices/approve", note: "device approval by pairing code (admin, /verify page)" },
  { method: "POST", path: "/api/v1/peers/register", note: "lease issuance (60–90 s TTL)" },
  { method: "POST", path: "/api/v1/peers/heartbeat", note: "lease extension + revocation notices" },
  { method: "POST", path: "/api/v1/peers/drain", note: "exclude peer from lookups" },
  { method: "POST", path: "/api/v1/peers/challenge/start", note: "hosting challenge" },
  { method: "POST", path: "/api/v1/peers/challenge/complete", note: "record timings, mark passed" },
  { method: "POST", path: "/api/v1/peers/lease", note: "eligibility lease issuance (ADR-012); response carries the swarm balance view (ADR-041)" },
  { method: "GET", path: "/api/v1/peers?profile_id=…", note: "peer lookup, limit ≤ 50" },
  { method: "POST", path: "/api/v1/rendezvous/offer", note: "signaling relay" },
  { method: "POST", path: "/api/v1/rendezvous/answer", note: "signaling relay" },
  { method: "GET", path: "/api/v1/rendezvous/pending", note: "mailbox drain (delete-on-read)" },
  { method: "POST", path: "/api/v1/events/job-result", note: "receipt digest + outcome + the counter-signed receipt (ADR-030)" },
  { method: "POST", path: "/api/v1/receipt", note: "canonical receipt intake" },
  { method: "POST", path: "/api/v1/session-authorize", note: "cooperative session metadata" },
  { method: "POST", path: "/api/v1/audit", note: "audit-epoch sweep (admin)" },
  { method: "POST", path: "/api/v1/admin/catalog/candidates", note: "candidate insert (admin)" },
  { method: "POST", path: "/api/v1/admin/catalog/promote", note: "candidate → active (admin)" },
];

export default function ReferencePage() {
  return (
    <main id="content">
      <div className="top" style={{ minHeight: "3.2rem" }}>
        <div className="wrap">
          <nav aria-label="Site">
            <span className="brand">MSP//HUB</span>
            <span className="spacer" />
            <a className="navlink" href="/">home</a>
            <a className="navlink" href="/api/v1/health">status</a>
          </nav>
        </div>
      </div>

      <div className="wrap" style={{ maxWidth: "60rem", margin: "0 auto", padding: "2.4rem 1.2rem 4rem" }}>
        <h1 style={{ fontSize: "1.6rem" }}>Reference</h1>
        <p className="note" style={{ marginBottom: "2rem" }}>
          The one rule, the trade, execution presets, and the tracker API
          surface — moved here from the main page (2026-10-17/18). The
          protocol spec lives in the repository under protocol/msp-v1.md.
        </p>

        <section id="rule" aria-labelledby="rule-heading">
          <h2 id="rule-heading">The one rule</h2>
          <div className="rule">
            <b>Host to consume — or stake a cloud key.</b> A peer earns swarm
            access by verifiably hosting the models it uses, or by serving a
            cloud model through its own provider key (DeepSeek or OpenRouter
            — joining with just a key lands in the next app release).
            Contribution is checked, not paid for: no credits, no payments,
            and prompts and completions never touch this site.
          </div>
        </section>

        <section id="trade" aria-labelledby="trade-heading">
          <h2 id="trade-heading">The trade</h2>
          <div className="rule">
            <b>One to one, token for token.</b> You may pull from the
            swarm up to what your own hardware and model can serve — a
            capacity measured when you prove hosting, not a debt earned
            from others using you. Whether or not anyone is pulling from
            your model right now never matters: your pull budget is your
            verified serving capacity, refilled continuously. The rule
            ships in watch mode first — budgets are measured and shown
            before they bind.
          </div>
        </section>

        <section id="presets" aria-labelledby="presets-heading">
          <h2 id="presets-heading">Presets</h2>
          <div className="tblcard">
            <table>
              <caption className="sr-only">Execution presets</caption>
              <thead>
                <tr>
                  <th scope="col">Preset</th>
                  <th scope="col">What runs</th>
                  <th scope="col">Correctness</th>
                </tr>
              </thead>
              <tbody>
                {PRESETS.map((p) => (
                  <tr key={p.name}>
                    <td>{p.name}</td>
                    <td>{p.allows}</td>
                    <td>{p.correctness}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="note">
            Swarm modes engage only when measured to beat the fastest eligible
            single host — otherwise automatic fallback. Accuracy benefits
            (better answers, even when slower) and mixed-model work stay
            behind their own correctness gates.
          </p>
        </section>

        <section id="api" aria-labelledby="api-heading">
          <h2 id="api-heading">Tracker API (v1)</h2>
          <div className="tblcard">
            <table>
              <caption className="sr-only">Implemented tracker API endpoints</caption>
              <tbody>
                {ENDPOINTS.map((e) => (
                  <tr key={`${e.method} ${e.path}`}>
                    <td>
                      {e.method} {e.path}
                    </td>
                    <td>{e.note}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      </div>
      <footer>
        ModelSwarm Protocol · source:{" "}
        <a href="https://github.com/ex4n1m0/modelswarm-protocol">github.com/ex4n1m0/modelswarm-protocol</a>{" "}
        · control plane only — never carries prompts, completions, or model
        files (ADR-001) · protocol v1
      </footer>
    </main>
  );
}
