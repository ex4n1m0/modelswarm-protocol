// Strict zod schemas for every hub request body (msp-v1 §3 + Phase B
// endpoints from DESIGN.md). `.strict()` everywhere: unknown keys — including
// inference-shaped fields like `messages`/`prompt`/`input` — are rejected
// with invalid_body (acceptance F4; ADR-001 content-blindness).
//
// Manifest schema mirrors catalog/schema-v2.json. One deliberate deviation,
// documented in the Phase B handoff: `runtime.name` is validated as a bounded
// identifier pattern instead of the schema's const value, so the tracker
// source never embeds the runtime's name (acceptance H2: no inference-runtime
// references in apps/tracker). The strict const is enforced by the CI
// golden-vector gate (scripts/validate-vectors.mjs via catalog/schema-v2.json).

import { z } from "zod";

const Hex32 = z.string().regex(/^[0-9a-f]{64}$/, "64 lowercase hex chars");
const ProfileId = z.string().regex(/^msp1:[0-9a-f]{64}$/, "msp1:<64 hex>");
const Base58Id = z.string().regex(/^[1-9A-HJ-NP-Za-km-z]{8,64}$/, "base58 id");
const Multiaddr = z
  .string()
  .min(1)
  .max(256)
  .regex(/^\/(ip4|ip6|dns)[A-Za-z0-9._:/-]*$/i, "multiaddr must start with /ip4, /ip6, or /dns");

export const DeviceStartSchema = z
  .object({
    installationId: Base58Id,
    pubKey: z.string().regex(/^[1-9A-HJ-NP-Za-km-z]{43,64}$/, "base58 ed25519 public key"),
  })
  .strict();

export const DeviceCompleteSchema = z
  .object({
    deviceCode: z.string().min(16).max(128),
  })
  .strict();

export const DeviceApproveSchema = z
  .object({
    // Format of randomUserCode(): 8 chars, no separators, ambiguous glyphs removed.
    userCode: z.string().regex(/^[ABCDEFGHJKMNPQRSTUVWXYZ23456789]{8}$/, "user pairing code (8 characters, e.g. AB2CDEFG)"),
  })
  .strict();

export const PeerRegisterSchema = z
  .object({
    peerId: Base58Id,
    addresses: z.array(Multiaddr).min(1).max(16),
    profiles: z.array(ProfileId).min(1).max(16),
    maxSlots: z.number().int().min(1).max(8),
    runtime: z
      .object({
        name: z.string().min(1).max(64),
        build: z.string().min(1).max(64),
      })
      .strict(),
  })
  .strict();

export const HeartbeatSchema = z
  .object({
    leaseId: z.string().regex(/^[0-9a-f]{32}$/, "128-bit hex lease id"),
    activeProfiles: z.array(ProfileId).min(0).max(16),
    freeSlots: z.number().int().min(0).max(8),
    queueMs: z.number().int().min(0).max(3_600_000),
    draining: z.boolean(),
  })
  .strict();

export const LeaseIdSchema = z
  .object({
    leaseId: z.string().regex(/^[0-9a-f]{32}$/, "128-bit hex lease id"),
  })
  .strict();

export const ChallengeStartSchema = z
  .object({
    leaseId: z.string().regex(/^[0-9a-f]{32}$/),
    profileId: ProfileId,
  })
  .strict();

export const ChallengeCompleteSchema = z
  .object({
    leaseId: z.string().regex(/^[0-9a-f]{32}$/),
    profileId: ProfileId,
    challengeId: z.string().regex(/^[0-9a-f]{32}$/),
    timings: z
      .object({
        firstTokenMs: z.number().int().min(0).max(3_600_000),
        totalMs: z.number().int().min(0).max(3_600_000),
      })
      .strict(),
  })
  .strict();

/** POST /peers/lease — consolidated eligibility-lease issuance (ADR-012). */
export const PeerLeaseSchema = z
  .object({
    leaseId: z.string().regex(/^[0-9a-f]{32}$/),
    profileId: ProfileId,
  })
  .strict();

export const RendezvousOfferSchema = z
  .object({
    toPeerId: Base58Id,
    offer: z.string().min(1).max(4096),
  })
  .strict();

export const RendezvousAnswerSchema = z
  .object({
    toPeerId: Base58Id,
    answer: z.string().min(1).max(4096),
  })
  .strict();

export const JobResultSchema = z
  .object({
    receiptDigest: z.string().regex(/^[0-9a-f]{64}$/, "sha256 hex digest"),
    outcome: z.enum(["stop", "length", "cancelled", "error"]),
  })
  .strict();

/** Phase B metadata-only endpoints. */
export const SessionAuthorizeSchema = z
  .object({
    peer_ids: z.array(Base58Id).min(1).max(16),
    profile_id: ProfileId,
    mode: z.string().regex(/^[a-z][a-z0-9_]{0,23}$/),
  })
  .strict();

export const AuditSchema = z
  .object({
    peer_ids: z.array(Base58Id).min(1).max(1000),
  })
  .strict();

// -- catalog admin -----------------------------------------------------------

export const ManifestSchema = z
  .object({
    schema_version: z.literal(2),
    hf_repo: z.string().regex(/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/),
    hf_revision: z.string().regex(/^[0-9a-f]{40}$/),
    artifact_hashes: z
      .array(
        z
          .object({
            path: z.string().regex(/^[A-Za-z0-9._/-]+$/).max(200),
            sha256: Hex32,
          })
          .strict(),
      )
      .min(1)
      .max(64),
    tokenizer_hash: Hex32,
    chat_template_hash: Hex32,
    architecture_hash: Hex32,
    quantization: z
      .object({
        method: z.string().regex(/^[A-Za-z0-9_]+$/).max(24),
        bits: z.number().int().min(1).max(8),
      })
      .strict(),
    runtime: z
      .object({
        // Pattern rather than the schema-v2 const: see file header + H2.
        name: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9._-]{0,39}$/),
        version: z.string().min(1).max(40),
        build_hash: Hex32,
      })
      .strict(),
    decoding_abi_version: z.number().int().min(1).max(255),
    speculative_capabilities: z
      .array(z.enum(["proposal_tokens", "logprobs", "batch_verify", "tree_attention"]))
      .max(8),
  })
  .strict();

export const CandidateSchema = z
  .object({
    manifest: ManifestSchema,
    display_name: z.string().min(1).max(80),
    status: z.enum(["candidate", "deprecated"]).optional(),
    provenance: z
      .object({
        resolvedBy: z.string().max(80).optional(),
        resolvedAt: z.string().datetime({ offset: true }).optional(),
        reviewedBy: z.string().max(80).optional(),
        licenseEvidenceUrl: z.string().url().max(300).optional(),
      })
      .strict()
      .optional(),
  })
  .strict();

export const PromoteSchema = z
  .object({
    profileId: ProfileId,
  })
  .strict();

// -- community model requests (ADR-023) --------------------------------------
// Exactly what a resolver run needs and the tracker can verify — every field
// is an artifact pointer filled by the desktop from the public HF Hub API.
// Content-blind by construction: no field can carry prompt/completion data.
export const ModelRequestSchema = z
  .object({
    hf_repo: z.string().regex(/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/).max(120),
    hf_revision: z.string().regex(/^[0-9a-f]{40}$/),
    artifact_path: z.string().regex(/^[A-Za-z0-9._/-]+$/).max(200),
    artifact_sha256: Hex32,
    artifact_bytes: z.number().int().min(1).max(200_000_000_000),
    // Resolver re-derives both from the artifact itself; these are display hints.
    quant_method: z.string().regex(/^[A-Za-z0-9_]+$/).max(24),
    quant_bits: z.number().int().min(1).max(8),
    display_name: z.string().min(1).max(80),
    note: z.string().max(280).optional(),
  })
  .strict();

export const ModelRequestResolveSchema = z
  .object({
    id: z.number().int().min(1),
    resolution: z.enum(["promoted", "rejected"]),
  })
  .strict();
