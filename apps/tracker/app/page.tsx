const DOWNLOAD = {
  file: "modelswarm-node-windows-x64.exe",
  url: "/downloads/modelswarm-node-windows-x64.exe",
  sums: "/downloads/SHA256SUMS.txt",
  version: "0.1.0",
  commit: "207279f",
  size: "6.3 MB",
  sha256: "7ead2a5020214e87ba55c65da212270cd3bb8d2249e5fd0bbf397148f64a7628",
};

const ENDPOINTS: Array<{ method: string; path: string; note: string }> = [
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
  { method: "POST", path: "/api/v1/receipt", note: "canonical receipt intake" },
  { method: "POST", path: "/api/v1/session-authorize", note: "cooperative session metadata" },
  { method: "POST", path: "/api/v1/audit", note: "audit-epoch sweep (admin)" },
  { method: "POST", path: "/api/v1/admin/catalog/candidates", note: "candidate insert (admin)" },
  { method: "POST", path: "/api/v1/admin/catalog/promote", note: "candidate → active (admin)" },
];

const css = `
  :root { color-scheme: dark; }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { background: #050a1f; color: #e8f4ff; font-family: ui-monospace, "Cascadia Code", "JetBrains Mono", Consolas, monospace; line-height: 1.6; }
  a { color: #2fd4ff; }
  .wrap { max-width: 1080px; margin: 0 auto; padding: 0 1.25rem; }
  nav { display: flex; align-items: center; gap: 1rem; padding: 0.9rem 0; font-size: 0.85rem; }
  nav .brand { color: #e8f4ff; font-weight: 700; letter-spacing: 0.08em; }
  nav .spacer { flex: 1; }
  .badge { border: 1px solid #2fd4ff55; color: #2fd4ff; border-radius: 999px; padding: 0.1rem 0.65rem; font-size: 0.72rem; letter-spacing: 0.1em; }
  .hero { width: 100%; height: auto; display: block; }
  .tagline { text-align: center; color: #9fb8d8; padding: 1.4rem 1rem 0.2rem; font-size: 0.95rem; }
  .tagline b { color: #e8f4ff; font-weight: 600; }
  section { padding: 2.2rem 0 0.6rem; }
  h2 { font-size: 0.85rem; letter-spacing: 0.22em; text-transform: uppercase; color: #2fd4ff; margin-bottom: 1.1rem; }
  .card { border: 1px solid #1c2b52; background: #0a1030; border-radius: 14px; padding: 1.5rem; }
  .dl { display: flex; flex-wrap: wrap; gap: 1.5rem; align-items: center; justify-content: space-between; }
  .dl h3 { font-size: 1.15rem; color: #e8f4ff; }
  .dl .meta { color: #9fb8d8; font-size: 0.8rem; margin-top: 0.35rem; }
  .btn { display: inline-block; background: linear-gradient(135deg, #1e6fff, #2fd4ff); color: #04101f; font-weight: 700; text-decoration: none; padding: 0.8rem 1.6rem; border-radius: 10px; font-size: 0.95rem; }
  .btn:hover { filter: brightness(1.12); }
  .btn.ghost { background: transparent; border: 1px solid #2fd4ff66; color: #2fd4ff; font-weight: 500; margin-left: 0.6rem; padding: 0.72rem 1.1rem; }
  .sum { font-size: 0.72rem; color: #7d95b8; margin-top: 0.9rem; word-break: break-all; }
  .warn { border: 1px solid #f5a62355; color: #ffc766; border-radius: 10px; padding: 0.7rem 0.9rem; font-size: 0.78rem; margin-top: 1rem; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(230px, 1fr)); gap: 1rem; }
  .feat { border: 1px solid #1c2b52; background: #0a1030; border-radius: 12px; padding: 1rem 1.1rem; font-size: 0.85rem; color: #c7d8ef; }
  .feat b { display: block; color: #e8f4ff; margin-bottom: 0.35rem; font-size: 0.9rem; }
  .rule { border-left: 3px solid #2fd4ff; background: #0a1030; padding: 0.9rem 1.1rem; border-radius: 0 10px 10px 0; color: #c7d8ef; font-size: 0.88rem; margin-bottom: 1.2rem; }
  code { color: #a855f7; font-size: 0.85em; }
  table { width: 100%; border-collapse: collapse; font-size: 0.8rem; color: #9fb8d8; }
  td { padding: 0.28rem 0.6rem 0.28rem 0; border-bottom: 1px solid #101a38; }
  td:first-child { color: #2fd4ff; white-space: nowrap; }
  footer { color: #7d95b8; font-size: 0.75rem; padding: 2.4rem 0 2rem; text-align: center; }
`;

export default function Home() {
  return (
    <main>
      <style dangerouslySetInnerHTML={{ __html: css }} />
      <div className="wrap">
        <nav aria-label="Site">
          <span className="brand">MSP//HUB</span>
          <span className="badge" title="Tracker status: check /api/v1/health">TRACKER LIVE</span>
          <span className="spacer" />
          <a href="/api/v1/health">status</a>
          <a href="/api/v1/catalog">catalog</a>
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

      <p className="tagline">
        Network-aware <b>micro-swarms</b> of peers hosting the same immutable model profile —<br />
        lossless cooperative inference with automatic <b>fastest-host fallback</b>.
      </p>

      <div className="wrap">
        <section aria-labelledby="download-heading">
          <h2 id="download-heading">Download the client</h2>
          <div className="card">
            <div className="dl">
              <div>
                <h3>ModelSwarm Node — Windows x64</h3>
                <p className="meta">
                  v{DOWNLOAD.version} · build {DOWNLOAD.commit} · {DOWNLOAD.size} ·
                  serves an OpenAI-compatible gateway on 127.0.0.1:11435 · run{" "}
                  <code>modelswarm-node --help</code> after download
                </p>
              </div>
              <div>
                <a
                  className="btn"
                  href={DOWNLOAD.url}
                  download={DOWNLOAD.file}
                  aria-label={`Download ModelSwarm Node version ${DOWNLOAD.version} for Windows x64`}
                >
                  Download .exe
                </a>
                <a className="btn ghost" href={DOWNLOAD.sums} download="SHA256SUMS.txt">
                  checksums
                </a>
              </div>
            </div>
            <p className="sum">sha256&nbsp; {DOWNLOAD.sha256}</p>
            <div className="warn" role="note">
              INTERNAL TEST BUILD — UNSIGNED. Windows SmartScreen will warn about an
              unverified publisher; choose “More info → Run anyway” only after verifying
              the SHA-256 above. Cooperative modes in this build are test-labeled and
              make no performance claims (ADR-019).
            </div>
          </div>
        </section>

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
              <b>Content-blind hub</b>
              This site&rsquo;s tracker handles metadata, leases and rendezvous only.
              Prompts and completions never touch it — asserted against the
              production database.
            </div>
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

        <footer>
          ModelSwarm Protocol · control plane only — never carries prompts,
          completions, or model files (ADR-001) · protocol v1
        </footer>
      </div>
    </main>
  );
}
