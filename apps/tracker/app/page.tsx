// Landing page copy contract (2026-10-09 simplification, owner directive:
// "simple with what is coming; keep what is needed to continue development"):
// - forward-looking: presets + roadmap; no downloads, no chat, no changelog
//   (installers remain available at their direct /api/download/<file> URLs)
// - every status label must stay honest: only FAST is live today
// - no payments/credits surface (AGENTS.md constraint 8)
// - copy never names the inference engine or model-file format — the H2 meta
//   test's source scan covers app/ marketing copy too

// Protocol-level execution presets (master prompt §3 / gap-engineering study §2).
const PRESETS: Array<{ name: string; status: "live" | "design"; allows: string; correctness: string }> = [
  { name: "FAST", status: "live", allows: "single host", correctness: "lossless — the baseline" },
  { name: "BALANCED", status: "design", allows: "single + hedged (≤3 raced peers)", correctness: "lossless" },
  { name: "DEEP", status: "design", allows: "BALANCED + best-of-n + deliberation roles", correctness: "approximate — never lossless" },
  { name: "VERIFIED", status: "design", allows: "BALANCED + audited chain", correctness: "detection-oriented, carries audit proofs" },
  { name: "MAXIMUM", status: "design", allows: "sanctioned composition of gated modes", correctness: "union of component labels" },
];

// Roadmap rows: status must reflect plan-of-record state, never aspiration.
const ROADMAP: Array<{ item: string; status: "dev" | "research" | "gated"; detail: string }> = [
  { item: "NAT traversal + circuit relay", status: "dev", detail: "Relay implementation verified on a real LAN; production relay hosting and honest direct-vs-relay measurements are next." },
  { item: "Hostile-network hardening", status: "dev", detail: "Lease-gated serving is live — every serving session carries a hub-signed lease. Adversarial verification and audit epochs extend it." },
  { item: "Cooperative inference", status: "research", detail: "Exact speculative decoding and verified token trees, engaged only when predicted to beat the measured fastest single host — otherwise automatic fallback." },
  { item: "Verified-work accounting + fair queueing", status: "research", detail: "Contribution made visible without becoming a currency; a minimal reputation chassis with measured detection math. No credits, no payments." },
  { item: "Deliberation + multi-model federation", status: "gated", detail: "Solvers, skeptics, and judges across different model swarms — gated behind its own correctness contracts so exact-profile guarantees never weaken." },
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
  { method: "POST", path: "/api/v1/peers/lease", note: "eligibility lease issuance (ADR-012)" },
  { method: "GET", path: "/api/v1/peers?profile_id=…", note: "peer lookup, limit ≤ 50" },
  { method: "POST", path: "/api/v1/rendezvous/offer", note: "signaling relay" },
  { method: "POST", path: "/api/v1/rendezvous/answer", note: "signaling relay" },
  { method: "GET", path: "/api/v1/rendezvous/pending", note: "mailbox drain (delete-on-read)" },
  { method: "POST", path: "/api/v1/events/job-result", note: "receipt digest + outcome" },
  { method: "POST", path: "/api/v1/receipt", note: "canonical receipt intake" },
  { method: "POST", path: "/api/v1/session-authorize", note: "cooperative session metadata" },
  { method: "POST", path: "/api/v1/audit", note: "audit-epoch sweep (admin)" },
  { method: "POST", path: "/api/v1/admin/catalog/candidates", note: "candidate insert (admin)" },
  { method: "POST", path: "/api/v1/admin/catalog/promote", note: "candidate → active (admin)" },
];

const css = `
  :root { color-scheme: dark; }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  /* #00060f = modal backdrop of hero.png (edges #00040a–#000811) so the artwork bleeds into the page */
  body { background: #00060f; color: #e8f4ff; font-family: ui-monospace, "Cascadia Code", "JetBrains Mono", Consolas, monospace; line-height: 1.6; }
  a { color: #2fd4ff; }
  .wrap { max-width: 1080px; margin: 0 auto; padding: 0 1.25rem; }
  nav { display: flex; align-items: center; gap: 1rem; padding: 0.9rem 0; font-size: 0.85rem; flex-wrap: wrap; }
  nav .brand { color: #e8f4ff; font-weight: 700; letter-spacing: 0.08em; }
  nav .spacer { flex: 1; }
  .badge { border: 1px solid #2fd4ff55; color: #2fd4ff; border-radius: 999px; padding: 0.1rem 0.65rem; font-size: 0.72rem; letter-spacing: 0.1em; }
  .hero { width: 75%; height: auto; display: block; margin: 0 auto; }
  .tagline { text-align: center; color: #9fb8d8; padding: 1.4rem 1rem 0.2rem; font-size: 0.95rem; }
  .tagline b { color: #e8f4ff; font-weight: 600; }
  .lede { text-align: center; color: #b6cbe8; padding: 1.6rem 1rem 0; font-size: 0.92rem; line-height: 1.7; max-width: 62rem; margin: 0 auto; }
  section { padding: 2.2rem 0 0.6rem; }
  h2 { font-size: 0.85rem; letter-spacing: 0.22em; text-transform: uppercase; color: #2fd4ff; margin-bottom: 1.1rem; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(230px, 1fr)); gap: 1rem; }
  .feat { border: 1px solid #1c2b52; background: #0a1030; border-radius: 12px; padding: 1rem 1.1rem; font-size: 0.85rem; color: #c7d8ef; }
  .feat b { display: block; color: #e8f4ff; margin-bottom: 0.35rem; font-size: 0.9rem; }
  .rule { border-left: 3px solid #2fd4ff; background: #0a1030; padding: 0.9rem 1.1rem; border-radius: 0 10px 10px 0; color: #c7d8ef; font-size: 0.88rem; margin-bottom: 1.2rem; }
  code { color: #a855f7; font-size: 0.85em; }
  table { width: 100%; border-collapse: collapse; font-size: 0.8rem; color: #9fb8d8; }
  td, th { padding: 0.28rem 0.6rem 0.28rem 0; border-bottom: 1px solid #101a38; text-align: left; }
  td:first-child { color: #2fd4ff; white-space: nowrap; }
  th { color: #7d95b8; font-size: 0.7rem; letter-spacing: 0.12em; text-transform: uppercase; }
  .pill { display: inline-block; border-radius: 999px; padding: 0.05rem 0.6rem; font-size: 0.68rem; letter-spacing: 0.1em; white-space: nowrap; }
  .pill.live { border: 1px solid #43c78f66; color: #7fe0b8; }
  .pill.dev { border: 1px solid #43c78f66; color: #7fe0b8; }
  .pill.design, .pill.research, .pill.gated { border: 1px solid #7d95b855; color: #9fb8d8; }
  .road-row { border: 1px solid #1c2b52; background: #0a1030; border-radius: 12px; padding: 1rem 1.2rem; margin-bottom: 0.9rem; }
  .road-row .rhead { display: flex; align-items: baseline; gap: 0.8rem; flex-wrap: wrap; }
  .road-row .rname { color: #e8f4ff; font-weight: 600; font-size: 0.95rem; }
  .road-row .rdetail { color: #c7d8ef; font-size: 0.85rem; margin-top: 0.4rem; }
  .note { font-size: 0.78rem; color: #7d95b8; margin: 0 0 1rem; }
  @media (max-width: 720px) { table { font-size: 0.72rem; } td:first-child { white-space: normal; } }
  footer { color: #7d95b8; font-size: 0.75rem; padding: 2.4rem 0 2rem; text-align: center; }
`;

const STATUS_LABEL: Record<string, string> = {
  live: "LIVE",
  dev: "IN DEVELOPMENT",
  research: "RESEARCH",
  gated: "GATED",
  design: "DESIGN",
};

export default function Home() {
  return (
    <main>
      <style dangerouslySetInnerHTML={{ __html: css }} />
      <div className="wrap">
        <nav aria-label="Site">
          <span className="brand">MSP//HUB</span>
          <span className="badge" title="Tracker status: check /api/v1/health">TRACKER LIVE</span>
          <span className="badge" id="online-badge" title="Peers with a live lease right now (GET /api/v1/stats)" style={{ borderColor: "#43c78f55", color: "#7fe0b8" }}>… online</span>
          <span className="spacer" />
          <a href="/api/v1/health">status</a>
          <a href="/api/v1/catalog">catalog</a>
          <a href="https://github.com/ex4n1m0/modelswarm-protocol" aria-label="ModelSwarm Protocol source code on GitHub">github</a>
        </nav>
      </div>

      {/* The hero artwork carries its own title lockup — full-bleed, no overlay text. */}
      <img
        src="/hero.png"
        alt="ModelSwarm — Decentralized Cooperative Inference Protocol: a glowing network of peer nodes converging into a central verification tree above a laptop"
        className="hero"
        width={1672}
        height={941}
      />

      <p className="lede">
        ModelSwarm is a decentralized inference protocol that groups peers hosting identical,
        cryptographically verified model profiles into latency-aware micro-swarms. It uses direct
        P2P communication, exact speculative decoding, parallel candidate search, and adversarial
        verification to improve inference performance, while automatically falling back to the
        fastest individual peer when distributed execution offers no advantage.
      </p>

      <p className="tagline">
        Network-aware <b>micro-swarms</b> of peers hosting the same immutable model profile —<br />
        lossless cooperative inference with automatic <b>fastest-host fallback</b>.
      </p>

      <div className="wrap">
        <section aria-labelledby="what-heading">
          <h2 id="what-heading">What it is</h2>
          <div className="rule">
            A peer may consume swarm inference for a model profile only while it is
            actively and verifiably <b>hosting the exact same profile</b> — same
            artifact hash, revision, quantization, and runtime. Enforced at the
            tracker, at the requester, and at every serving peer.
          </div>
          <div className="grid">
            <div className="feat">
              <b>Exact-profile swarms</b>
              Identity is a manifest-derived <code>msp1:</code> id — tokenizer, prompt
              template, runtime build and artifact digest all pinned. Different hash,
              different swarm.
            </div>
            <div className="feat">
              <b>Lossless cooperation</b>
              Speculative decoding and verified token trees reproduce the target
              model&rsquo;s output exactly — proven token-for-token, including under
              adversarial peers.
            </div>
            <div className="feat">
              <b>Honest routing</b>
              Cooperative modes engage only when predicted to beat the measured
              fastest eligible single host — and fall back automatically when they
              don&rsquo;t.
            </div>
            <div className="feat">
              <b>Reciprocity, not currency</b>
              You earn swarm access by hosting the same profile you consume —
              challenges, leases and receipts make contribution verifiable. No
              credits, tokens, or payments.
            </div>
            <div className="feat">
              <b>Content-blind hub</b>
              This site&rsquo;s tracker handles metadata, leases and rendezvous only.
              Prompts and completions never touch it — asserted against the
              production database.
            </div>
            <div className="feat">
              <b>Adversarial resistance</b>
              Every serving session carries a hub-signed lease bound to the
              QUIC-authenticated peer identity; stale or foreign leases are refused
              before the first token.
            </div>
          </div>
        </section>

        <section aria-labelledby="coming-heading">
          <h2 id="coming-heading">What&rsquo;s coming</h2>
          <p className="note">
            The direction, in one table and five rows. A preset is a protocol-level
            contract, not a UI preference: it fixes which execution modes may run, the
            cohort cap, and the correctness label the swarm must honor — with the
            compute budget declared on the wire. FAST is the default and live today;
            the others activate exactly when their component modes pass their
            correctness gates.
          </p>
          <table>
            <caption className="visually-hidden" style={{ position: "absolute", left: "-9999px" }}>
              Execution presets and their status
            </caption>
            <thead>
              <tr>
                <th scope="col">Preset</th>
                <th scope="col">Status</th>
                <th scope="col">Modes allowed</th>
                <th scope="col">Correctness label</th>
              </tr>
            </thead>
            <tbody>
              {PRESETS.map((p) => (
                <tr key={p.name}>
                  <td>{p.name}</td>
                  <td><span className={"pill " + p.status}>{STATUS_LABEL[p.status]}</span></td>
                  <td>{p.allows}</td>
                  <td>{p.correctness}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div style={{ marginTop: "1.4rem" }}>
            {ROADMAP.map((r) => (
              <div key={r.item} className="road-row">
                <div className="rhead">
                  <span className="rname">{r.item}</span>
                  <span className={"pill " + r.status}>{STATUS_LABEL[r.status]}</span>
                </div>
                <p className="rdetail">{r.detail}</p>
              </div>
            ))}
          </div>
        </section>

        <section aria-labelledby="api-heading">
          <h2 id="api-heading">Tracker API (v1)</h2>
          <table>
            <caption className="visually-hidden" style={{ position: "absolute", left: "-9999px" }}>
              Implemented tracker API endpoints
            </caption>
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
        </section>

          <script
          dangerouslySetInnerHTML={{
            __html: `
(async () => {
  const badge = document.getElementById('online-badge');
  const update = async () => {
    try {
      const r = await fetch('/api/v1/stats');
      if (!r.ok) throw 0;
      const { peersOnline } = await r.json();
      badge.textContent = peersOnline + (peersOnline === 1 ? ' peer online' : ' peers online');
    } catch { badge.textContent = '— online'; }
  };
  await update(); setInterval(update, 30000);
})();`,
          }}
        />
      <footer>
          ModelSwarm Protocol · source:{" "}
          <a href="https://github.com/ex4n1m0/modelswarm-protocol">github.com/ex4n1m0/modelswarm-protocol</a>{" "}
          · control plane only — never carries prompts, completions, or model
          files (ADR-001) · protocol v1
        </footer>
      </div>
    </main>
  );
}
