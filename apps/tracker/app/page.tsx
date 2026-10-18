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
//   census is the first section under the lede (before "How it works")
// - 2026-10-15 visual redesign (visual layer only): dark-navy palette
//   (#080C14 page / #111927 surfaces / cyan #38D9F5 accent), sans body with
//   mono reserved for labels/code, restyled header/tables/footer, skip link.
//   Download URLs, the online-badge/census script and its DOM ids
//   (online-badge, census-body) are unchanged on every retarget.
// - 2026-10-17 usability pass (owner directive "improve so that it is much
//   easier to understand and to use"): the shared design system moved to
//   app/globals.css (fixes /reference, which referenced these classes but
//   rendered browser-default — dark on dark); a REAL visible h1 + subhead +
//   CTA band under the hero artwork (the only headline was baked into the
//   image and the only CTA was a small nav button); a "How it works" section
//   (three honest steps, present tense for what ships today) AFTER the
//   census; downloads restyled as OS-labeled buttons with Windows primary;
//   census gains a 4 s honest fallback so a stalled fetch shows "census
//   unavailable" instead of "loading…" forever. Same copy rules apply:
//   honest tense, no engine/format names, no payments/credits, census first
//   under the lede, ids online-badge/census-body preserved.

// Presets + the tracker API table moved to /reference (owner directive
// 2026-10-17: keep the main page minimal).

export default function Home() {
  return (
    <main>
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

        {/* The hero artwork carries its own title lockup; the band below it
            adds the readable headline + call to action the image cannot
            provide (screen readers and first-time visitors both need it). */}
        <img
          src="/hero.png"
          alt="ModelSwarm — Decentralized Cooperative Inference Protocol: a glowing network of peer nodes converging into a central verification tree above a laptop"
          className="hero"
          width={1672}
          height={941}
        />
        <div className="hero-band">
          <h1>Your computers, one AI swarm.</h1>
          <p>
            Install the app and pick a model — your machine hosts it, and
            peers running the <b>exact same model</b> cross-check your
            answers. No accounts, no payments: hosting is what earns you
            swarm access.
          </p>
          <div className="cta-row">
            <a className="btn" href="#get">Get the app</a>
            <a className="btn ghost" href="#how">How it works</a>
          </div>
        </div>
      </div>

      <div className="wrap" id="content" tabIndex={-1}>
        <p className="lede">
          Coming next: mixed models working together as <b>solvers, skeptics,
          and judges</b> — behind their own correctness gates.
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

        <section id="how" aria-labelledby="how-heading">
          <h2 id="how-heading">How it works</h2>
          <div className="steps">
            <div className="step">
              <span className="n">STEP 1</span>
              <h3>Install the app</h3>
              <p>
                One download for Windows, macOS, or Linux. From then on the
                app verifies and installs its own updates — this is the
                last manual download.
              </p>
            </div>
            <div className="step">
              <span className="n">STEP 2</span>
              <h3>Join a swarm</h3>
              <p>
                Pick a model and press Join — it downloads and hosts on
                your machine, or serve a cloud model through your own
                provider key. Either way your machine counts as one peer.
              </p>
            </div>
            <div className="step">
              <span className="n">STEP 3</span>
              <h3>Ask anything</h3>
              <p>
                Chat in the app or through the local API on the same
                machine. Peers hosting the exact same model cross-check
                every reply — slower by design, verified by cohort.
              </p>
            </div>
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
            <b>v0.2.39</b> — test builds, unsigned. This build keeps
            itself current: it checks for updates on start and every six
            hours, verifies each update against a key built into the
            app, and restarts to install — this is the last manual
            download. Same-model cooperation (faster or
            verified-accurate answers, never worse than the
            fastest single host) works between two peers on a direct or LAN
            network; the app reports honest status for anything not engaged.
            <div className="dl-grid">
              <a className="dl primary" href="/api/download/ModelSwarm-Setup-0.2.39-windows-x64.exe">
                <span className="os">Windows</span>
                <span className="fmt">installer · .exe · x64</span>
              </a>
              <a className="dl" href="/api/download/ModelSwarm-0.2.39-macos-arm64.dmg">
                <span className="os">macOS</span>
                <span className="fmt">Apple Silicon · .dmg</span>
              </a>
              <a className="dl" href="/api/download/ModelSwarm-0.2.39-linux-amd64.deb">
                <span className="os">Linux</span>
                <span className="fmt">.deb · amd64</span>
              </a>
              <a className="dl" href="/api/download/ModelSwarm-0.2.39-linux-amd64.AppImage">
                <span className="os">Linux</span>
                <span className="fmt">AppImage · self-updating</span>
              </a>
            </div>
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
  // Honest fallback: if the first poll has not landed within 4 s, say so
  // instead of showing "loading…" forever (a late success still replaces
  // this on arrival — the 30 s refresh keeps running either way).
  setTimeout(() => {
    if (census.textContent === 'loading…') census.textContent = 'census unavailable — still trying…';
    if (badge.textContent === '… online') badge.textContent = '— online';
  }, 4000);
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
