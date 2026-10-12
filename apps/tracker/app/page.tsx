// Landing page copy contract (2026-10-11 second revision, owner directive:
// "no one uses today's solution, the client is not hosted on the website —
// remove old-plan references, focus on the future plan, 10x less text"):
// - future-focused copy only: swarms, cooperation, accuracy, mixed models
// - no live/status labels, no client/download references, no "today" hedges
//   (honesty rides on tense + the engage/fallback note, not status pills)
// - dev-facing surfaces stay: API table, status/catalog links, online badge
// - no payments/credits surface (AGENTS.md constraint 8)
// - copy never names the inference engine or model-file format — the H2 meta
//   test's source scan covers app/ marketing copy too
// - installers remain available at their direct /api/download/<file> URLs
//   (2026-10-12 revision, owner directive "release to the website, I will
//   get people to test it": a minimal honest download section is restored
//   below — same copy rules apply: no engine/format names, honest tense)

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
  .lede { text-align: center; color: #b6cbe8; padding: 1.6rem 1rem 0; font-size: 0.95rem; line-height: 1.7; max-width: 56rem; margin: 0 auto; }
  .lede b { color: #e8f4ff; font-weight: 600; }
  section { padding: 2.2rem 0 0.6rem; }
  h2 { font-size: 0.85rem; letter-spacing: 0.22em; text-transform: uppercase; color: #2fd4ff; margin-bottom: 1.1rem; }
  .rule { border-left: 3px solid #2fd4ff; background: #0a1030; padding: 0.9rem 1.1rem; border-radius: 0 10px 10px 0; color: #c7d8ef; font-size: 0.9rem; margin-bottom: 1.2rem; }
  .rule b { color: #e8f4ff; }
  code { color: #a855f7; font-size: 0.85em; }
  table { width: 100%; border-collapse: collapse; font-size: 0.8rem; color: #9fb8d8; }
  td, th { padding: 0.28rem 0.6rem 0.28rem 0; border-bottom: 1px solid #101a38; text-align: left; }
  td:first-child { color: #2fd4ff; white-space: nowrap; }
  th { color: #7d95b8; font-size: 0.7rem; letter-spacing: 0.12em; text-transform: uppercase; }
  .note { font-size: 0.78rem; color: #7d95b8; margin: 1rem 0 0; max-width: 46rem; }
  @media (max-width: 720px) { table { font-size: 0.72rem; } td:first-child { white-space: normal; } }
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
        ModelSwarm pools computers hosting verified models into <b>swarms</b> —
        same-model swarms cooperate for <b>faster or better</b> answers; later,
        mixed models work together as <b>solvers, skeptics, and judges</b>.
        Host to participate.
      </p>

      <div className="wrap">
        <section aria-labelledby="rule-heading">
          <h2 id="rule-heading">The one rule</h2>
          <div className="rule">
            <b>Host to consume.</b> A peer earns swarm access only by
            verifiably hosting the models it uses — contribution is checked,
            not paid for. No credits, no payments. Prompts and completions
            never touch this site.
          </div>
        </section>

        <section aria-labelledby="presets-heading">
          <h2 id="presets-heading">Presets</h2>
          <table>
            <caption className="visually-hidden" style={{ position: "absolute", left: "-9999px" }}>
              Execution presets
            </caption>
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
          <p className="note">
            Swarm modes engage only when measured to beat the fastest eligible
            single host — otherwise automatic fallback. Accuracy benefits
            (better answers, even when slower) and mixed-model work stay
            behind their own correctness gates.
          </p>
        </section>

        <section aria-labelledby="get-heading">
          <h2 id="get-heading">Desktop app</h2>
          <div className="rule">
            <b>v0.2.21</b> — test builds, unsigned. Same-model cooperation
            (faster or verified-accurate answers, never worse than the
            fastest single host) works between two peers on a direct or LAN
            network; the app reports honest status for anything not engaged.
            <ul style={{ margin: "0.8rem 0 0", padding: "0 0 0 1.1rem" }}>
              <li>
                <a href="/api/download/ModelSwarm-Setup-0.2.21-windows-x64.exe">
                  Windows installer (.exe)
                </a>
              </li>
              <li>
                <a href="/api/download/ModelSwarm-0.2.21-linux-amd64.deb">Linux (.deb)</a>
                {" · "}
                <a href="/api/download/ModelSwarm-0.2.21-linux-amd64.AppImage">
                  AppImage
                </a>
              </li>
              <li>
                <a href="/api/download/ModelSwarm-0.2.21-macos-arm64.dmg">
                  macOS (Apple Silicon, .dmg)
                </a>
              </li>
            </ul>
            <p className="note" style={{ marginTop: "0.6rem" }}>
              First hosting start enrolls the device (auto-approved under
              the test cap) and downloads the chosen model from its original
              source. Checksums: /downloads/SHA256SUMS.txt.
            </p>
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

        <section aria-labelledby="census-heading">
          <h2 id="census-heading">Live swarm census</h2>
          <div className="rule">
            <span id="census-body" style={{ fontSize: "0.95rem" }}>
              loading…
            </span>
            <p className="note" style={{ marginTop: "0.5rem" }}>
              Models other peers are hosting right now — picking one of
              these means the swarm can cooperate from your first session.
            </p>
          </div>
        </section>

          <script
          dangerouslySetInnerHTML={{
            __html: `
(async () => {
  const badge = document.getElementById('online-badge');
  const census = document.getElementById('census-body');
  const update = async () => {
    let peersOnline;
    try {
      const r = await fetch('/api/v1/stats');
      if (!r.ok) throw 0;
      ({ peersOnline } = await r.json());
      badge.textContent = peersOnline + (peersOnline === 1 ? ' peer online' : ' peers online');
    } catch { badge.textContent = '— online'; }
    try {
      const [s, c] = await Promise.all([fetch('/api/v1/stats'), fetch('/api/v1/catalog')]);
      if (!s.ok || !c.ok) throw 0;
      const stats = await s.json();
      const cat = await c.json();
      const names = new Map(
        (cat.models || cat.profiles || []).map((m) => [m.profile_id, m.display_name]),
      );
      const rows = (stats.models || [])
        .map((row) => ({ name: names.get(row.profileId) || row.profileId.slice(0, 18) + '…', peers: row.peers }))
        .sort((a, b) => b.peers - a.peers || a.name.localeCompare(b.name));
      if (!rows.length) {
        census.textContent = 'No peers hosting yet — install the app and start hosting to be the first.';
      } else {
        census.replaceChildren(
          ...rows.map((row) => {
            const el = document.createElement('span');
            el.textContent = row.name + ' · ' + row.peers + (row.peers === 1 ? ' peer' : ' peers');
            el.style.cssText = 'display:inline-block;margin:0 0.6rem 0.4rem 0;padding:0.15rem 0.6rem;border:1px solid #ffffff22;border-radius:999px;font-size:0.9rem;white-space:nowrap;';
            return el;
          }),
        );
      }
    } catch { census.textContent = 'census unavailable'; }
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
