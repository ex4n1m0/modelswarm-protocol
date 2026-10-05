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

## Current environment (preview-grade — by design)

- **No `DATABASE_URL`** → in-memory store: registrations/leases work but
  reset on cold starts. Fine for smoke tests and protocol demos; not for
  real nodes.
- **Ephemeral `HUB_SIGNING_KEY`** → regenerated per boot; a catalog
  signature verifies only against the instance that signed it. Real
  clients pin the hub public key, so this must become a stable env secret
  before any node trusts the catalog.
- **No `ADMIN_TOKEN`** → admin endpoints return 403 (catalog promotion is
  blocked until set — matching the catalog-stays-empty policy).

## To reach production grade (owner actions)

1. Create a managed Postgres (Neon via Vercel Marketplace recommended) and
   set `DATABASE_URL`; run `npm run db:migrate` against it.
2. Generate a stable Ed25519 hub keypair; set `HUB_SIGNING_KEY` (and
   publish/pin the public key in the node builds).
3. Set `ADMIN_TOKEN` when the first catalog profile is ready for
   promotion.
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
