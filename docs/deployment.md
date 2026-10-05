# Deployment — modelswarm.deepflux.space

Status: **live (preview-grade)** since 2026-10-05 · Vercel team
`timedivision`, project `tracker` (rename to `modelswarm-tracker` pending,
dashboard setting). Owner-authorized this date ("the domain is
modelswarm.deepflux.space you can create it").

## What exists

| Piece | State |
|---|---|
| Domain | `modelswarm.deepflux.space` added to project; auto-assigned to the latest production deployment |
| DNS | `deepflux.space` is on Vercel nameservers (`ns1/ns2.vercel-dns.com`); subdomain CNAME → `cname.vercel-dns.com` (record `rec_a161ab026428db3317cdaf28`) — created via CLI, no external DNS provider involved |
| TLS | Vercel-managed, verified live (`/api/v1/health` → 200 over HTTPS) |
| App | `apps/tracker` (Next.js); verified live: health (frozen body), signed catalog envelope, 401 on unauthenticated register |
| Preview alias | https://tracker-tau-three.vercel.app (same deployment) |

## Current environment (production-partial since 2026-10-05)

- **Stable `MSP_HUB_SEED` set (production)** — the tracker now signs every
  catalog with the pinned identity; verified live: the catalog signature
  cryptographically verifies against `apps/tracker/keys/hub-public.hex`
  and fails against a wrong key. Rotation procedure: new seed → new env →
  update the pinned file → release → coordinated catalog bump.
- **`ADMIN_TOKEN` set (production)** — admin endpoints active; verified
  live (no token → 403; valid token + invalid body → 400 `invalid_body`).
  Owner holds the token; rotate via `vercel env remove/add` + redeploy.
- **No `DATABASE_URL` yet** → in-memory store: registrations/leases work
  but reset on cold starts. The ONLY remaining owner step (below).

## To reach production grade (owner actions)

1. **Create the Neon database** (the one remaining step):
   neon.tech → sign up → New Project (`modelswarm`) → copy the pooled
   connection string → either set it yourself
   (`cd apps/tracker && printf '%s' "URL" | vercel env add DATABASE_URL
   production && vercel deploy --prod`) or paste it to the engineer, who
   runs the migration and verifies.
2. ~~Stable hub key~~ done (pinned at `apps/tracker/keys/hub-public.hex`).
3. ~~`ADMIN_TOKEN`~~ done.
4. Optional: connect the GitHub repo to the Vercel project for
   push-to-deploy (currently deploys are manual via CLI).

Set env vars with: `cd apps/tracker && vercel env add NAME production` +
redeploy (`vercel deploy --prod`).

## Boundaries that still hold

- The tracker is and remains **content-blind** (ADR-001): no prompts,
  completions, model files, or HF tokens ever reach it — enforced by
  schema absence, tested in CI.
- Domain/DNS changes beyond this subdomain remain owner-gated per the
  plans' stop rules.
