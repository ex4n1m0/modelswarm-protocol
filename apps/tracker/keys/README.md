# Hub key pinning

`hub-public.hex` is the Ed25519 public key of the production tracker
signing identity (derived from the `MSP_HUB_SEED` secret, which lives only
in Vercel env). Catalog consumers (nodes) MUST verify `GET /api/v1/catalog`
signatures against this key (canonical-JSON payload
{catalogVersion, generatedAt, profiles}; rules = msp-v1 §2.2). Rotating the
seed requires updating this file, cutting a release, and a coordinated
catalog-version bump — see docs/deployment.md.
