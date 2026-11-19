"use client";

// /account/login — website-account sign-in / sign-up form. Posts to the
// /api/v1/account/* APIs with fetch(credentials: "same-origin") so the
// HttpOnly msw_session cookie set on success rides the same origin; the
// JSON error envelope {error:{code,message}} is shown inline. On success
// the browser is sent to /account (the dashboard).

import { useState } from "react";

type Mode = "signin" | "signup";

export default function AccountLoginPage() {
  const [mode, setMode] = useState<Mode>("signin");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const path = mode === "signin" ? "/api/v1/account/login" : "/api/v1/account/signup";
      const body =
        mode === "signin"
          ? { email: email.trim(), password }
          : { email: email.trim(), password, displayName: displayName.trim() };
      const res = await fetch(path, {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      if (res.ok) {
        window.location.assign("/account");
        return;
      }
      const parsed = (await res.json().catch(() => null)) as { error?: { code: string; message: string } } | null;
      const code = parsed?.error?.code;
      setError(
        code === "signup_closed"
          ? "Account creation is currently closed by the operator. Try signing in instead."
          : (parsed?.error?.message ?? `request failed (${res.status})`),
      );
    } catch {
      setError("network error — try again");
    }
    setBusy(false);
  }

  return (
    <main className="acct-shell">
      <form onSubmit={submit} className="acct-card" style={{ width: "min(92vw, 460px)" }}>
        <a href="/" style={{ color: "var(--accent)", fontSize: "0.8rem", textDecoration: "none" }}>← modelswarm.deepflux.space</a>
        <h1 className="acct-title" style={{ margin: "0.9rem 0 0.4rem" }}>
          {mode === "signin" ? "Sign in" : "Create account"}
        </h1>
        <p style={{ color: "var(--text-2)", fontSize: "0.85rem", lineHeight: 1.6, margin: "0 0 1.1rem" }}>
          {mode === "signin"
            ? "Manage the machines you enrolled from the desktop app."
            : "An account groups your installations so the site can show “your machines”."}
        </p>

        {mode === "signup" && (
          <label className="acct-label">
            DISPLAY NAME
            <input
              className="acct-input"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              placeholder="Ada"
              autoComplete="name"
              required
              maxLength={64}
            />
          </label>
        )}

        <label className="acct-label">
          EMAIL
          <input
            className="acct-input"
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="you@example.com"
            autoComplete="email"
            required
            maxLength={254}
          />
        </label>

        <label className="acct-label" style={{ marginBottom: 14 }}>
          PASSWORD
          <input
            className="acct-input"
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={mode === "signup" ? "at least 10 characters" : ""}
            autoComplete={mode === "signin" ? "current-password" : "new-password"}
            required
            minLength={mode === "signup" ? 10 : undefined}
            maxLength={128}
          />
        </label>

        <button type="submit" className="btn" disabled={busy} style={{ width: "100%", border: 0, cursor: busy ? "default" : "pointer", opacity: busy ? 0.6 : 1 }}>
          {busy ? "working…" : mode === "signin" ? "sign in" : "create account"}
        </button>

        {error && (
          <p role="alert" style={{ margin: "14px 0 0", fontSize: "0.82rem", color: "#E07A7A" }}>
            {error}
          </p>
        )}

        <p style={{ margin: "1rem 0 0", fontSize: "0.82rem", color: "var(--text-3)" }}>
          {mode === "signin" ? "No account yet? " : "Already have an account? "}
          <button
            type="button"
            className="btn ghost"
            style={{ padding: "0.25rem 0.7rem", fontSize: "0.78rem" }}
            onClick={() => {
              setMode(mode === "signin" ? "signup" : "signin");
              setError(null);
            }}
          >
            {mode === "signin" ? "Create one" : "Sign in"}
          </button>
        </p>
      </form>
    </main>
  );
}
