"use client";

// /verify — the device-approval half of msp-v1 §3.2 enrollment. A desktop
// client that starts hosting shows a pairing code (it polls
// /auth/device/complete); the owner enters that code here with the ops
// admin token to let the device into the roster. Knowledge of the code
// alone is NOT enough — the admin token keeps the gate owner-controlled.
import { useState } from "react";

const CODE_HINT = "8 characters, shown in the app under Setup → Device approval";

export default function VerifyPage() {
  const [userCode, setUserCode] = useState("");
  const [adminToken, setAdminToken] = useState("");
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setResult(null);
    try {
      const res = await fetch("/api/v1/admin/devices/approve", {
        method: "POST",
        headers: { "content-type": "application/json", "x-msp-admin": adminToken },
        body: JSON.stringify({ userCode: userCode.trim().toUpperCase() }),
      });
      const body = await res.json().catch(() => null);
      if (res.ok) {
        setResult({ ok: true, text: "Device approved. The client joins the roster within ~30 s — no restart needed." });
        setUserCode("");
      } else {
        const message =
          body?.error?.message ?? (res.status === 403 ? "admin token required" : "approval failed");
        setResult({ ok: false, text: `${message} (${res.status})` });
      }
    } catch {
      setResult({ ok: false, text: "network error — try again" });
    }
    setBusy(false);
  }

  return (
    <main style={{ background: "#00060f", color: "#e8f4ff", minHeight: "100vh", display: "flex", alignItems: "center", justifyContent: "center", fontFamily: 'ui-monospace, "Cascadia Code", Consolas, monospace' }}>
      <form
        onSubmit={submit}
        style={{ width: "min(92vw, 480px)", border: "1px solid #1c2b52", background: "#0a1030", borderRadius: 14, padding: "1.6rem" }}
      >
        <a href="/" style={{ color: "#2fd4ff", fontSize: "0.8rem", textDecoration: "none" }}>← modelswarm.deepflux.space</a>
        <h1 style={{ fontSize: "1.05rem", letterSpacing: "0.12em", textTransform: "uppercase", color: "#2fd4ff", margin: "0.9rem 0 0.4rem" }}>
          Approve a device
        </h1>
        <p style={{ color: "#9fb8d8", fontSize: "0.85rem", lineHeight: 1.6, margin: "0 0 1.1rem" }}>
          A new ModelSwarm installation joins the swarm only after approval. The
          client shows its pairing code while waiting — enter it below with the
          owner token to approve it.
        </p>

        <label style={{ display: "block", fontSize: "0.72rem", letterSpacing: "0.1em", color: "#9fb8d8", marginBottom: 4 }}>
          PAIRING CODE
          <input
            value={userCode}
            onChange={(e) => setUserCode(e.target.value)}
            placeholder="AB2CDEFG"
            autoComplete="off"
            required
            maxLength={8}
            style={{ width: "100%", marginTop: 4, background: "#00060f", color: "#e8f4ff", border: "1px solid #1c2b52", borderRadius: 8, padding: "0.6rem 0.7rem", fontSize: "1rem", letterSpacing: "0.2em", fontFamily: "inherit" }}
          />
        </label>
        <p style={{ color: "#5c729a", fontSize: "0.7rem", margin: "4px 0 12px" }}>{CODE_HINT}</p>

        <label style={{ display: "block", fontSize: "0.72rem", letterSpacing: "0.1em", color: "#9fb8d8", marginBottom: 14 }}>
          OWNER TOKEN
          <input
            type="password"
            value={adminToken}
            onChange={(e) => setAdminToken(e.target.value)}
            autoComplete="off"
            required
            style={{ width: "100%", marginTop: 4, background: "#00060f", color: "#e8f4ff", border: "1px solid #1c2b52", borderRadius: 8, padding: "0.6rem 0.7rem", fontSize: "0.9rem", fontFamily: "inherit" }}
          />
        </label>

        <button
          type="submit"
          disabled={busy}
          style={{ width: "100%", background: "#2fd4ff", color: "#001018", border: 0, borderRadius: 8, padding: "0.65rem", fontWeight: 700, fontSize: "0.9rem", cursor: busy ? "default" : "pointer", opacity: busy ? 0.6 : 1 }}
        >
          {busy ? "approving…" : "approve device"}
        </button>

        {result && (
          <p role="status" style={{ margin: "14px 0 0", fontSize: "0.82rem", color: result.ok ? "#43c78f" : "#e06565" }}>
            {result.text}
          </p>
        )}
      </form>
    </main>
  );
}
