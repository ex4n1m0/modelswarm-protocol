// Every installer bundles the full pinned inference engine; the model
// weights download from HuggingFace on first use. v0.2.17: GPU compute
// (Vulkan) — Windows bundles a second pinned engine that offloads to any
// NVIDIA/AMD/Intel GPU with honest CPU fallback; v0.2.16: one-click model
// switch + stop (swap keeps the old swarm serving during download);
// v0.2.15: community model requests (ADR-023) — search Hugging Face in
// the app; v0.2.14: zero-click enrollment + registration fixes.
// Same version on every OS = same build.
const DOWNLOADS = [
  { os: "Windows x64", file: "ModelSwarm-Setup-0.2.17-windows-x64.exe", note: "SmartScreen will warn (unsigned) — More info → Run anyway. WebView2 installs automatically if missing." },
  { os: "Linux amd64 (.deb)", file: "ModelSwarm-0.2.17-linux-amd64.deb", note: "sudo apt install ./modelswarm…deb — webkit dependencies are declared and pulled in." },
  { os: "Linux amd64 (AppImage)", file: "ModelSwarm-0.2.17-linux-amd64.AppImage", note: "chmod +x then run; self-contained except webkit2gtk (in every mainstream distro)." },
  { os: "macOS Apple Silicon (.dmg)", file: "ModelSwarm-0.2.17-macos-arm64.dmg", note: "Unsigned: right-click → Open the first time, or xattr -cr /Applications/ModelSwarm.app." },
];
const DOWNLOAD = {
  version: "0.2.17",
  sums: "/downloads/SHA256SUMS.txt",
  sha256: "b7d9172dc7c0cc78572c9333f52723fa50e1af0a169ba43b0789977a95c5a8ba",
};

// Dedicated community room — the room word is fixed here and nowhere else on
// this page, so the site never offers a room choice. The chat is OnlyHumans'
// sealed service; the browser talks to it directly, never through the tracker.
const CHAT_JOIN = "https://onlyhumans.deepflux.space/join#room=ModelSwarm";
const CHAT_SERVICE = "https://onlyhumans.deepflux.space";


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
  nav { display: flex; align-items: center; gap: 1rem; padding: 0.9rem 0; font-size: 0.85rem; }
  nav .brand { color: #e8f4ff; font-weight: 700; letter-spacing: 0.08em; }
  nav .spacer { flex: 1; }
  .badge { border: 1px solid #2fd4ff55; color: #2fd4ff; border-radius: 999px; padding: 0.1rem 0.65rem; font-size: 0.72rem; letter-spacing: 0.1em; }
  .hero { width: 75%; height: auto; display: block; margin: 0 auto; }
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
  .models { display: flex; flex-direction: column; gap: 10px; }
  .model-row { border: 1px solid #1c2b52; background: #0a1030; border-radius: 10px; padding: 12px 14px; display: flex; justify-content: space-between; gap: 12px; align-items: center; }
  .model-row .mname { font-weight: 600; }
  .model-row .mmeta { font-size: 0.72rem; color: #9fb8d8; font-family: ui-monospace, Consolas, monospace; margin-top: 2px; word-break: break-all; }
  .mcount { font-family: ui-monospace, Consolas, monospace; font-size: 0.8rem; color: #7fe0b8; border: 1px solid #43c78f55; border-radius: 999px; padding: 2px 10px; white-space: nowrap; }
  .chat-meta { color: #9fb8d8; font-size: 0.8rem; margin-bottom: 0.9rem; }
  .chat-meta b { color: #e8f4ff; }
  .chat-frame { width: 100%; height: 640px; border: 1px solid #1c2b52; border-radius: 12px; background: #0e1518; display: block; }
  @media (max-width: 720px) { .chat-frame { height: 540px; } }
  .chat-note { font-size: 0.72rem; color: #7d95b8; margin-top: 0.7rem; }
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

      <p className="tagline">
        Network-aware <b>micro-swarms</b> of peers hosting the same immutable model profile —<br />
        lossless cooperative inference with automatic <b>fastest-host fallback</b>.
      </p>

      <div className="wrap">
        <section aria-labelledby="download-heading">
          <h2 id="download-heading">Download the client — v{DOWNLOAD.version}, all platforms, everything bundled</h2>
          <div className="card">
            <div className="dl">
              <div>
                <h3>ModelSwarm Desktop — {DOWNLOAD.version} (internal test)</h3>
                <p className="meta">
                  windowed app hosting one of three model profiles: model-picker first page,
                  download-or-load on selection, swarm starts, local OpenAI-compatible API +
                  test box. The pinned inference engine ships inside every installer; the
                  model weights download from HuggingFace on first use — never bundled.
                  First hosting start asks for a one-time device approval
                  at <a href="/verify">/verify</a> (owner-controlled).
                </p>
              </div>
              <div>
                {DOWNLOADS.map((d) => (
                  <a key={d.file} className="btn ghost" data-file={d.file} style={{ marginRight: 8, marginBottom: 6, display: "inline-block" }} href={"/api/download/" + d.file} download={d.file} aria-label={"Download ModelSwarm " + DOWNLOAD.version + " for " + d.os}>
                    {d.os} · v{DOWNLOAD.version} <span className="dl-count" data-count-for={d.file} style={{ opacity: 0.75 }}>· ↓0</span>
                  </a>
                ))}
                <a className="btn ghost" href={DOWNLOAD.sums} download="SHA256SUMS.txt">checksums</a>
              </div>
            </div>
            <p className="sum">windows sha256&nbsp; {DOWNLOAD.sha256}</p>
            <div className="warn" role="note">
              INTERNAL TEST BUILD — UNSIGNED. Verify the SHA-256 above before running; expect
              OS warnings (SmartScreen / Gatekeeper) for an unverified publisher. Same version
              number on every OS = same build. Per-OS first-run notes:
              <ul style={{ margin: "8px 0 0", paddingLeft: 18, fontSize: "0.78rem" }}>
                {DOWNLOADS.map((d) => (
                  <li key={d.file} style={{ marginBottom: 4 }}><b>{d.os}:</b> {d.note}</li>
                ))}
              </ul>
            </div>
          </div>
        </section>

        <section aria-labelledby="models-h">
          <h2 id="models-h">Models · live swarm census</h2>
          <div className="models" id="model-rows" aria-live="polite">
            <p className="sum">loading catalog…</p>
          </div>
          <p className="sum" style={{ marginTop: 10 }}>
            Counts are installations with a live lease right now (GET /api/v1/stats);
            each model is its own exact-profile swarm.
          </p>
        </section>

        <section aria-labelledby="chat-h">
          <h2 id="chat-h">Community chat · dedicated ModelSwarm room</h2>
          <div className="card">
            <p className="chat-meta">
              Type a name to join — the room word is preset to this site&rsquo;s
              dedicated <b>ModelSwarm</b> room. No other room is offered here.
            </p>
            <iframe
              className="chat-frame"
              src={CHAT_JOIN}
              title="ModelSwarm community chat (OnlyHumans)"
              loading="lazy"
            />
            <p className="chat-note">
              Sealed end-to-end by{" "}
              <a href={CHAT_SERVICE} target="_blank" rel="noopener noreferrer">OnlyHumans</a>{" "}
              — a separate service from the tracker; chat content never touches the
              ModelSwarm control plane.{" "}
              <a href={CHAT_JOIN} target="_blank" rel="noopener noreferrer">Open the room in a new tab</a>
            </p>
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

          <script
          dangerouslySetInnerHTML={{
            __html: `
(async () => {
  const badge = document.getElementById('online-badge');
  const rows = document.getElementById('model-rows');
  let names = null;
  const catalogNames = async () => {
    if (names) return names;
    const c = await fetch('/api/v1/catalog');
    if (!c.ok) throw 0;
    const env = await c.json();
    names = Object.fromEntries(env.profiles.map(p => [p.profile_id, p]));
    return names;
  };
  const update = async () => {
    try {
      const r = await fetch('/api/v1/stats');
      if (!r.ok) throw 0;
      const { peersOnline, models, downloads } = await r.json();
      badge.textContent = peersOnline + (peersOnline === 1 ? ' peer online' : ' peers online');
      const byProfile = Object.fromEntries((models || []).map(m => [m.profileId, m.peers]));
      const cat = await catalogNames();
      const counts = document.querySelectorAll('[data-count-for]');
      for (const el of counts) {
        const n = (downloads || {})[el.dataset.countFor] ?? 0;
        el.textContent = '· ↓' + n;
      }
      rows.innerHTML = Object.entries(cat).map(([id, p]) => {
        const n = byProfile[id] ?? 0;
        const quant = p.manifest?.quantization;
        return '<div class="model-row"><div><div class="mname">' + p.display_name + '</div>' +
          '<div class="mmeta">' + id + ' · ' + (quant ? quant.method + ' · ' + quant.bits + '-bit' : '') + ' · engine ' + (p.manifest?.runtime?.version || '') + '</div></div>' +
          '<span class="mcount">' + n + (n === 1 ? ' online' : ' online') + '</span></div>';
      }).join('');
    } catch { badge.textContent = '— online'; rows.innerHTML = '<p class="sum">census unavailable</p>'; }
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
