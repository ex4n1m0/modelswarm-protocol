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
// - 2026-10-12 owner directive "move the live swarm census to the top": the
//   census is the first section under the lede (before "The one rule")
// - 2026-10-15 visual redesign (visual layer only): dark-navy palette
//   (#080C14 page / #111927 surfaces / cyan #38D9F5 accent), sans body with
//   mono reserved for labels/code, restyled header/tables/footer, skip link
//   + sr-only H1. Copy, links, download URLs, the online-badge/census script
//   and its DOM ids (online-badge, census-body), and section order are
//   UNCHANGED. Download links carry v0.2.25 from release-train commit 96adf68
//   (the train parked this redesign mid-flight and restored the tree to HEAD;
//   this re-application folds its 0.2.24->0.2.25 retarget in).
// - 2026-10-15 (later): v0.2.26 retarget (release train — the language-pin
//   fix); 5 functional replacements, same copy rules.

// Presets + the tracker API table moved to /reference (owner directive
// 2026-10-17: keep the main page minimal).


const css = `
  :root {
    color-scheme: dark;
    --bg: #080C14;
    --bg-hero: #00060f; /* modal backdrop of hero.png (edges #00040a–#000811) so the artwork bleeds into the page */
    --surface: #111927;
    --surface-2: #162133;
    --text: #F3F6FC;
    --text-2: #B2BDCD;
    --text-3: #8CA0BC;
    --accent: #38D9F5;
    --accent-2: #A69BFF;
    --ok: #7FE0B8;
    --border: #24334C;
    --border-soft: #1B2638;
    --mono: ui-monospace, "Cascadia Code", "JetBrains Mono", Consolas, monospace;
  }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  html { scroll-behavior: smooth; }
  body { background: var(--bg); color: var(--text); font-family: system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif; line-height: 1.65; -webkit-font-smoothing: antialiased; }
  a { color: var(--accent); text-decoration: underline; text-decoration-color: rgba(56, 217, 245, 0.35); text-underline-offset: 3px; }
  a:hover { text-decoration-color: currentColor; }
  a:focus-visible { outline: 2px solid var(--accent-2); outline-offset: 2px; border-radius: 4px; }
  .skip { position: absolute; left: -9999px; top: 0; z-index: 10; background: var(--accent); color: #06131D; font-weight: 650; padding: 0.6rem 1.1rem; border-radius: 0 0 10px 0; text-decoration: none; }
  .skip:focus { left: 0; }
  .sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; border: 0; }
  .wrap { max-width: 1200px; margin: 0 auto; padding: 0 1.5rem; }
  /* Hero zone keeps the artwork's own backdrop pure under the whole image (85%) and blends into the page navy below. */
  .top { background: linear-gradient(180deg, var(--bg-hero) 0%, var(--bg-hero) 85%, var(--bg) 100%); }
  nav { display: flex; align-items: center; gap: 0.9rem; padding: 1.15rem 0 0.95rem; font-size: 0.86rem; flex-wrap: wrap; }
  nav .brand { color: var(--text); font-family: var(--mono); font-weight: 700; letter-spacing: 0.08em; margin-right: 0.25rem; }
  nav .spacer { flex: 1; }
  nav .navlink { color: var(--text-2); text-decoration: none; padding: 0.45rem 0.15rem; }
  nav .navlink:hover { color: var(--accent); }
  .btn { display: inline-block; background: var(--accent); color: #06131D; font-weight: 650; font-size: 0.85rem; border: 1px solid transparent; border-radius: 8px; padding: 0.5rem 1rem; text-decoration: none; }
  .btn:hover { background: #62E2F8; color: #06131D; }
  .badge { border: 1px solid rgba(56, 217, 245, 0.45); background: rgba(56, 217, 245, 0.08); color: var(--accent); border-radius: 999px; padding: 0.18rem 0.7rem; font-size: 0.72rem; letter-spacing: 0.1em; font-family: var(--mono); white-space: nowrap; }
  .hero { width: min(920px, 88%); height: auto; display: block; margin: 0 auto; }
  .lede { text-align: center; color: var(--text-2); padding: 2.1rem 1rem 0.2rem; font-size: 1.06rem; line-height: 1.78; max-width: 56rem; margin: 0 auto; }
  .lede b { color: var(--text); font-weight: 600; }
  section { padding: 2.6rem 0 0.2rem; scroll-margin-top: 1rem; }
  h2 { display: flex; align-items: center; gap: 1rem; font-family: var(--mono); font-size: 0.78rem; font-weight: 600; letter-spacing: 0.22em; text-transform: uppercase; color: var(--accent); margin-bottom: 1.15rem; }
  h2::after { content: ""; flex: 1; height: 1px; background: var(--border-soft); }
  .rule { background: var(--surface); border: 1px solid var(--border); border-left: 3px solid var(--accent); padding: 1.15rem 1.3rem; border-radius: 0 12px 12px 0; color: var(--text-2); font-size: 0.95rem; }
  .rule b { color: var(--text); }
  .rule ul { margin: 0.9rem 0 0; padding: 0 0 0 1.15rem; line-height: 1.9; }
  code { color: var(--accent-2); font-family: var(--mono); font-size: 0.85em; }
  .tblcard { background: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 0.35rem 1.25rem 0.55rem; }
  table { width: 100%; border-collapse: collapse; font-size: 0.85rem; color: var(--text-2); }
  td, th { padding: 0.5rem 0.9rem 0.5rem 0; border-bottom: 1px solid var(--border-soft); text-align: left; vertical-align: top; }
  tbody tr:last-child td { border-bottom: none; }
  td { transition: background-color 0.12s ease; }
  td:first-child { color: var(--accent); font-family: var(--mono); font-size: 0.8rem; white-space: nowrap; }
  th { color: var(--text-3); font-family: var(--mono); font-size: 0.68rem; letter-spacing: 0.14em; text-transform: uppercase; }
  .note { font-size: 0.8rem; color: var(--text-3); margin: 1rem 0 0; max-width: 46rem; line-height: 1.7; }
  footer { border-top: 1px solid var(--border-soft); margin-top: 3.2rem; padding: 2.2rem 0 2.6rem; color: var(--text-3); font-size: 0.78rem; text-align: center; line-height: 1.8; }
  footer a { color: var(--text-2); }
  @media (hover: hover) { tbody tr:hover td { background: var(--surface-2); } }
  @media (max-width: 720px) {
    .wrap { padding: 0 1.25rem; }
    table { font-size: 0.78rem; }
    td:first-child { white-space: normal; overflow-wrap: anywhere; }
  }
  @media (prefers-reduced-motion: reduce) {
    html { scroll-behavior: auto; }
    *, *::before, *::after { transition: none !important; }
  }
`;

export default function Home() {
  return (
    <main>
      <style dangerouslySetInnerHTML={{ __html: css }} />
      <a className="skip" href="#content">Skip to content</a>

      <div className="top">
        <div className="wrap">
          <nav aria-label="Site">
            <span className="brand">MSP//HUB</span>
            <span className="badge" title="Tracker status: check /api/v1/health">TRACKER LIVE</span>
            <span className="badge" id="online-badge" title="Peers with a live lease right now (GET /api/v1/stats)" style={{ borderColor: "rgba(127, 224, 184, 0.45)", background: "rgba(127, 224, 184, 0.08)", color: "#7FE0B8" }}>… online</span>
            <span className="spacer" />
            <a className="navlink" href="/reference">reference</a>
            <a className="navlink" href="/api/v1/health">status</a>
            <a className="navlink" href="/api/v1/catalog">catalog</a>
            <a className="navlink" href="https://github.com/ex4n1m0/modelswarm-protocol" aria-label="ModelSwarm Protocol source code on GitHub">github</a>
            <a className="btn" href="#get">Get started</a>
          </nav>
        </div>

        {/* The hero artwork carries its own title lockup — full-bleed, no overlay text. */}
        <h1 className="sr-only">ModelSwarm — Decentralized Cooperative Inference Protocol</h1>
        <img
          src="/hero.png"
          alt="ModelSwarm — Decentralized Cooperative Inference Protocol: a glowing network of peer nodes converging into a central verification tree above a laptop"
          className="hero"
          width={1672}
          height={941}
        />
      </div>

      <div className="wrap" id="content" tabIndex={-1}>
        <p className="lede">
          ModelSwarm pools computers hosting verified models into <b>swarms</b> —
          same-model swarms cooperate for <b>faster or better</b> answers; later,
          mixed models work together as <b>solvers, skeptics, and judges</b>.
          Host to participate.
        </p>

        <section id="census" aria-labelledby="census-heading">
          <h2 id="census-heading">Live swarm census</h2>
          <div className="rule">
            <span id="census-body" style={{ fontSize: "0.95rem" }}>
              loading…
            </span>
            <p className="note" style={{ marginTop: "0.5rem" }}>
              Models other peers are hosting right now — picking one of
              these means the swarm can cooperate from your first session.
            </p>
            <p className="note" id="clients-served" style={{ marginTop: "0.5rem" }}>
              &nbsp;
            </p>
          </div>
        </section>

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

        <section id="get" aria-labelledby="get-heading">
          <h2 id="get-heading">Desktop app</h2>
          <div className="rule">
            <b>v0.2.31</b> — test builds, unsigned. This build keeps
            itself current: it checks for updates on start and every six
            hours, verifies each update against a key built into the
            app, and restarts to install — this is the last manual
            download. Same-model cooperation (faster or
            verified-accurate answers, never worse than the
            fastest single host) works between two peers on a direct or LAN
            network; the app reports honest status for anything not engaged.
            <ul style={{ margin: "0.8rem 0 0", padding: "0 0 0 1.15rem" }}>
              <li>
                <a href="/api/download/ModelSwarm-Setup-0.2.31-windows-x64.exe">
                  Windows installer (.exe)
                </a>
              </li>
              <li>
                <a href="/api/download/ModelSwarm-0.2.31-linux-amd64.deb">Linux (.deb)</a>
                {" · "}
                <a href="/api/download/ModelSwarm-0.2.31-linux-amd64.AppImage">
                  AppImage (self-updating)
                </a>
              </li>
              <li>
                <a href="/api/download/ModelSwarm-0.2.31-macos-arm64.dmg">
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
      ({ peersOnline, clientsServed } = await r.json());
      badge.textContent = peersOnline + (peersOnline === 1 ? ' peer online' : ' peers online');
      const served = document.getElementById('clients-served');
      if (served && Number.isFinite(clientsServed)) {
        served.textContent = clientsServed + (clientsServed === 1
          ? ' client served so far (installer downloads and automatic updates, all operating systems, each count as one).'
          : ' clients served so far (installer downloads and automatic updates, all operating systems, each count as one).');
      }
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
            el.style.cssText = 'display:inline-block;margin:0 0.6rem 0.4rem 0;padding:0.2rem 0.7rem;border:1px solid rgba(56,217,245,0.35);background:#0D1524;border-radius:999px;font-size:0.9rem;white-space:nowrap;color:#DCE6F5;';
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
