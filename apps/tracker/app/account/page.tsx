"use client";

// /account — the signed-in dashboard: who you are, an "Add a machine" card
// (pairing code from the desktop app), and your machines table with Revoke
// and Rename. All refreshes are user-triggered — no polling, no timers.

import { useEffect, useState } from "react";

interface Me {
  userId: string;
  email: string;
  displayName: string;
  createdAt: string;
}

interface Machine {
  installationId: string;
  label: string;
  status: "active" | "revoked";
  addedAt: string;
  approvedAt: string | null;
  lastSeenAt: string | null;
}

/** Online window for the machine heartbeat's last-seen touch (0009). */
const ONLINE_WINDOW_MS = 120_000;

/** Liveness label, computed at render/refresh only — no timers. Online iff
 *  the machine heartbeated inside the window; otherwise minute-level
 *  relative "Seen …"; null = never heartbeated under this binding. */
function seenStatus(lastSeenAt: string | null, now: number): { text: string; online: boolean } {
  if (lastSeenAt === null) return { text: "Never", online: false };
  const ageMs = now - Date.parse(lastSeenAt);
  if (ageMs < ONLINE_WINDOW_MS) return { text: "Online", online: true };
  const mins = Math.max(1, Math.floor(ageMs / 60_000));
  if (mins < 60) return { text: `Seen ${mins}m ago`, online: false };
  const hours = Math.floor(mins / 60);
  if (hours < 24) return { text: `Seen ${hours}h ago`, online: false };
  return { text: `Seen ${Math.floor(hours / 24)}d ago`, online: false };
}

function shortId(id: string): string {
  return id.length > 14 ? `${id.slice(0, 10)}…` : id;
}

export default function AccountDashboardPage() {
  const [me, setMe] = useState<Me | null>(null);
  const [machines, setMachines] = useState<Machine[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [label, setLabel] = useState("");
  const [addBusy, setAddBusy] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ installationId: string; value: string } | null>(null);
  const [rowBusy, setRowBusy] = useState(false);

  async function load(): Promise<boolean> {
    const meRes = await fetch("/api/v1/account/me", { credentials: "same-origin" });
    if (meRes.status === 401) {
      window.location.assign("/account/login");
      return false;
    }
    if (!meRes.ok) {
      setLoadError(`could not load account (${meRes.status})`);
      return false;
    }
    setMe((await meRes.json()) as Me);
    const machinesRes = await fetch("/api/v1/account/machines", { credentials: "same-origin" });
    if (machinesRes.ok) {
      const body = (await machinesRes.json()) as { machines: Machine[] };
      setMachines(body.machines);
    }
    return true;
  }

  useEffect(() => {
    // One-shot initial load; every later refresh is user-triggered.
    void load();
  }, []);

  async function addMachine(e: React.FormEvent) {
    e.preventDefault();
    setAddBusy(true);
    setAddError(null);
    try {
      const res = await fetch("/api/v1/account/devices/approve", {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          userCode: code.trim().toUpperCase(),
          ...(label.trim() ? { label: label.trim() } : {}),
        }),
      });
      if (res.ok) {
        setCode("");
        setLabel("");
        await load();
      } else {
        const body = (await res.json().catch(() => null)) as { error?: { message: string } } | null;
        setAddError(body?.error?.message ?? `approval failed (${res.status})`);
      }
    } catch {
      setAddError("network error — try again");
    }
    setAddBusy(false);
  }

  async function revoke(installationId: string) {
    setRowBusy(true);
    try {
      const res = await fetch(`/api/v1/account/machines/${encodeURIComponent(installationId)}/revoke`, {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json" },
      });
      if (res.ok) await load();
      else setAddError(`revoke failed (${res.status})`);
    } catch {
      setAddError("network error — try again");
    }
    setRowBusy(false);
  }

  async function rename(installationId: string) {
    if (!renaming || renaming.installationId !== installationId) return;
    const next = renaming.value.trim();
    if (next.length === 0) {
      setRenaming(null);
      return;
    }
    setRowBusy(true);
    try {
      const res = await fetch(`/api/v1/account/machines/${encodeURIComponent(installationId)}/label`, {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ label: next }),
      });
      if (res.ok) {
        setRenaming(null);
        await load();
      } else {
        setAddError(`rename failed (${res.status})`);
      }
    } catch {
      setAddError("network error — try again");
    }
    setRowBusy(false);
  }

  async function logout() {
    await fetch("/api/v1/account/logout", {
      method: "POST",
      credentials: "same-origin",
      headers: { "content-type": "application/json" },
    }).catch(() => null);
    window.location.assign("/account/login");
  }

  return (
    <main className="acct-shell">
      <div className="acct-card" style={{ width: "min(94vw, 880px)", margin: "1.5rem 0" }}>
        <div style={{ display: "flex", alignItems: "baseline", gap: "0.8rem", flexWrap: "wrap" }}>
          <a href="/" style={{ color: "var(--accent)", fontSize: "0.8rem", textDecoration: "none" }}>← modelswarm.deepflux.space</a>
          <span style={{ flex: 1 }} />
          <button type="button" className="btn ghost" onClick={logout} style={{ fontSize: "0.78rem" }}>
            Log out
          </button>
        </div>

        <h1 className="acct-title" style={{ margin: "0.9rem 0 0.2rem" }}>Your machines</h1>
        {loadError && <p role="alert" style={{ color: "#E07A7A", fontSize: "0.85rem" }}>{loadError}</p>}
        {me && (
          <p style={{ color: "var(--text-2)", fontSize: "0.9rem", margin: "0 0 1.2rem" }}>
            {me.displayName} <span style={{ color: "var(--text-3)" }}>· {me.email}</span>
          </p>
        )}

        <form onSubmit={addMachine} className="acct-card" style={{ background: "var(--bg)", marginBottom: "1.2rem", padding: "1rem 1.1rem" }}>
          <h2 style={{ margin: "0 0 0.5rem" }}>Add a machine</h2>
          <p style={{ color: "var(--text-2)", fontSize: "0.8rem", margin: "0 0 0.8rem", lineHeight: 1.6 }}>
            Start hosting in the desktop app, then type the 8-character pairing code it shows here — the machine joins this account.
          </p>
          <div style={{ display: "flex", gap: "0.6rem", flexWrap: "wrap" }}>
            <input
              className="acct-input"
              value={code}
              onChange={(e) => setCode(e.target.value.toUpperCase())}
              placeholder="AB2CDEFG"
              autoComplete="off"
              required
              maxLength={8}
              style={{ width: "10rem", letterSpacing: "0.2em", fontFamily: "var(--mono)" }}
            />
            <input
              className="acct-input"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder="Label (optional)"
              maxLength={64}
              style={{ flex: 1, minWidth: "12rem" }}
            />
            <button type="submit" className="btn" disabled={addBusy} style={{ border: 0, cursor: addBusy ? "default" : "pointer", opacity: addBusy ? 0.6 : 1 }}>
              {addBusy ? "approving…" : "Add machine"}
            </button>
          </div>
          {addError && <p role="alert" style={{ margin: "0.7rem 0 0", fontSize: "0.82rem", color: "#E07A7A" }}>{addError}</p>}
        </form>

        <div className="tblcard">
          <table>
            <thead>
              <tr>
                <th>Label</th>
                <th>Installation</th>
                <th>Status</th>
                <th>Seen</th>
                <th>Added</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {machines.length === 0 && (
                <tr>
                  <td colSpan={6} style={{ color: "var(--text-3)" }}>
                    No machines yet — add one with its pairing code above.
                  </td>
                </tr>
              )}
              {machines.map((m) => {
                const seen = seenStatus(m.lastSeenAt, Date.now());
                return (
                <tr key={m.installationId}>
                  <td>
                    {renaming?.installationId === m.installationId ? (
                      <span
                        role="group"
                        onKeyDown={(e) => {
                          if (e.key === "Enter") void rename(m.installationId);
                          if (e.key === "Escape") setRenaming(null);
                        }}
                      >
                        <input
                          className="acct-input"
                          value={renaming.value}
                          onChange={(e) => setRenaming({ installationId: m.installationId, value: e.target.value })}
                          maxLength={64}
                          autoFocus
                          style={{ width: "9rem", padding: "0.3rem 0.5rem" }}
                        />
                        <button type="button" className="btn" disabled={rowBusy} onClick={() => void rename(m.installationId)} style={{ marginLeft: 6, padding: "0.3rem 0.7rem", fontSize: "0.75rem", border: 0 }}>
                          save
                        </button>
                        <button type="button" className="btn ghost" onClick={() => setRenaming(null)} style={{ marginLeft: 4, padding: "0.3rem 0.7rem", fontSize: "0.75rem" }}>
                          cancel
                        </button>
                      </span>
                    ) : (
                      m.label
                    )}
                  </td>
                  <td title={m.installationId}>{shortId(m.installationId)}</td>
                  <td>
                    <span className="badge" style={m.status === "revoked" ? { color: "var(--text-3)", borderColor: "var(--border)" } : undefined}>
                      {m.status}
                    </span>
                  </td>
                  <td style={{ whiteSpace: "nowrap", color: seen.online ? "var(--text)" : "var(--text-3)" }}>
                    <span
                      aria-hidden
                      style={{
                        display: "inline-block",
                        width: 8,
                        height: 8,
                        borderRadius: "999px",
                        marginRight: "0.45rem",
                        background: seen.online ? "var(--ok)" : "var(--text-3)",
                      }}
                    />
                    {seen.text}
                  </td>
                  <td>{new Date(m.addedAt).toLocaleDateString()}</td>
                  <td style={{ whiteSpace: "nowrap" }}>
                    {renaming?.installationId === m.installationId ? null : (
                      <>
                        <button
                          type="button"
                          className="btn ghost"
                          disabled={rowBusy}
                          onClick={() => setRenaming({ installationId: m.installationId, value: m.label })}
                          style={{ padding: "0.3rem 0.7rem", fontSize: "0.75rem" }}
                        >
                          Rename
                        </button>{" "}
                        <button
                          type="button"
                          className="btn ghost"
                          disabled={rowBusy || m.status === "revoked"}
                          onClick={() => void revoke(m.installationId)}
                          style={{ padding: "0.3rem 0.7rem", fontSize: "0.75rem" }}
                        >
                          Revoke
                        </button>
                      </>
                    )}
                  </td>
                </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
    </main>
  );
}
