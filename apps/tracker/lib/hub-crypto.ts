// Server-side PUBLIC-signature verification for the OnlyHumans hub port
// (ADR-013 D4, P3 slice 5). Faithful port of OnlyHumans-main/api/_mail-crypto.ts
// plus the reg/room canonicalizers from reg.ts / room.ts — zero-dependency
// (node:crypto SPKI wrap + minimal libp2p protobuf decode + URL-safe base64).
//
// D4 "untrusted storage" posture, clarified: the hub holds NO private keys and
// never reads/plaintexts the sealed `env_json` (it stays opaque ciphertext).
// Verifying PUBLIC Ed25519 signatures server-side is fully compatible with that
// and is the reference's proven posture — it stops forged registrations, room
// hijacks, and non-owner inbox drains BEFORE storage, while clients STILL
// re-verify every fetched record (hub.rs) and check the key→id binding again.
//
// Canonical preimages are byte-for-byte the pinned client's (onlyhumans-core
// hub.rs): UTF-8 of the pipe-delimited string WITH a trailing '|', ts_ms as a
// decimal integer, addrs joined by ','. Cross-checked against hub.rs
// canonical / room_canonical / mail_canonical / drain_canonical and the
// reference TS canonicalizers (they agree). The client encodes keys/sigs as
// URL-safe base64 WITHOUT padding (crypto.rs base64), keys as libp2p PublicKey
// protobuf, and peer ids as base58btc of the identity multihash of that
// protobuf — all mirrored here.

import { createPublicKey, verify as nodeVerify } from "node:crypto";
import type { HubMailItem, HubRegistration, HubRoomHost } from "@/lib/hub-schemas";

/** Ed25519 verify with zero dependencies: wrap the raw 32-byte public key in a
 *  fixed SPKI prefix and use node:crypto (mirrors reference _mail-crypto.ts). */
export function ed25519Verify(sig: Uint8Array, msg: Uint8Array, rawPub: Uint8Array): boolean {
  const spki = Buffer.alloc(44);
  Buffer.from("302a300506032b6570032100", "hex").copy(spki, 0);
  Buffer.from(rawPub).copy(spki, 12);
  try {
    const key = createPublicKey({ key: spki, format: "der", type: "spki" });
    return nodeVerify(null, Buffer.from(msg), key, Buffer.from(sig));
  } catch {
    return false;
  }
}

/** Minimal protobuf decode of a libp2p PublicKey:
 *  { required Type type = 1 (varint); required bytes data = 2 }.
 *  Ed25519 keys are type 1 with a 32-byte payload. */
export function libp2pEd25519Key(buf: Uint8Array): Uint8Array | null {
  let i = 0;
  let type = -1;
  let data: Uint8Array | null = null;
  while (i < buf.length) {
    const tag = buf[i++]!;
    const fieldNum = tag >> 3;
    const wireType = tag & 7;
    if (wireType === 0) {
      let v = 0;
      let sh = 0;
      for (;;) {
        const b = buf[i++]!;
        v |= (b & 0x7f) << sh;
        sh += 7;
        if (!(b & 0x80)) break;
      }
      if (fieldNum === 1) type = v;
    } else if (wireType === 2) {
      let len = 0;
      let sh = 0;
      for (;;) {
        const b = buf[i++]!;
        len |= (b & 0x7f) << sh;
        sh += 7;
        if (!(b & 0x80)) break;
      }
      const val = buf.subarray(i, i + len);
      i += len;
      if (fieldNum === 2) data = val;
    } else {
      return null; // unsupported wire type
    }
  }
  if (type !== 1 || !data || data.length !== 32) return null;
  return data;
}

/** URL-safe base64 WITHOUT padding (the client's crypto.rs base64 alphabet). */
export function b64decode(s: string): Uint8Array {
  const ALPHA = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  const vals: number[] = [];
  for (const c of s) {
    const idx = ALPHA.indexOf(c);
    if (idx < 0) throw new Error("bad b64 char");
    vals.push(idx);
  }
  const out: number[] = [];
  for (let j = 0; j < vals.length; j += 4) {
    const c = vals.slice(j, j + 4);
    let n = 0;
    for (let k = 0; k < c.length; k++) n |= c[k]! << (18 - 6 * k);
    out.push((n >> 16) & 0xff);
    if (c.length > 2) out.push((n >> 8) & 0xff);
    if (c.length > 3) out.push(n & 0xff);
  }
  return new Uint8Array(out);
}

/** libp2p PeerId of an Ed25519 PublicKey protobuf: base58btc of the identity
 *  multihash (0x00 <len> <protobuf>). Mirrors reference peerIdFromLibp2pKey
 *  (and tracker crypto.derivePeerId for the canonical 36-byte form). */
const B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
export function peerIdFromLibp2pKey(pubProtobuf: Uint8Array): string {
  const mh = new Uint8Array(2 + pubProtobuf.length);
  mh[0] = 0x00; // identity multihash
  mh[1] = pubProtobuf.length;
  mh.set(pubProtobuf, 2);
  let n = 0n;
  for (const b of mh) n = n * 256n + BigInt(b);
  let out = "";
  while (n > 0n) {
    out = B58_ALPHABET[Number(n % 58n)] + out;
    n /= 58n;
  }
  for (const b of mh) {
    if (b !== 0) break;
    out = "1" + out;
  }
  return out;
}

// ---------------------------------------------------------------------------
// Canonical preimages (byte-for-byte hub.rs / reference TS)

const enc = (s: string): Uint8Array => new TextEncoder().encode(s);

/** hub.rs `canonical` / reference reg.ts: OH1-reg|peer|pub|addrs,|ts| */
export function regCanonical(peerId: string, pubB64: string, addrs: string[], ts: number): Uint8Array {
  return enc(`OH1-reg|${peerId}|${pubB64}|${addrs.join(",")}|${ts}`);
}

/** hub.rs `room_canonical` / reference room.ts: OH1-room|room|host|pub|ts| */
export function roomCanonical(roomId: string, host: string, pubB64: string, ts: number): Uint8Array {
  return enc(`OH1-room|${roomId}|${host}|${pubB64}|${ts}`);
}

/** hub.rs `mail_canonical` / reference _mail-crypto.mailCanonical. */
export function mailCanonical(from: string, to: string, ts: number, envJson: string): Uint8Array {
  return enc(`OH1-mail-v1|${from}|${to}|${ts}|${envJson}`);
}

/** hub.rs `drain_canonical` / reference _mail-crypto.drainCanonical. */
export function drainCanonical(peer: string, ts: number): Uint8Array {
  return enc(`OH1-drain-v1|${peer}|${ts}`);
}

// ---------------------------------------------------------------------------
// High-level verifiers (compose decode + verify + binding, fail closed)

/** reg: signature verifies under the included key AND that key derives the
 *  claimed peer_id (reference reg.ts: !ok || derived !== peerId -> 403). */
export function registrationVerified(reg: HubRegistration): boolean {
  try {
    const pubProto = b64decode(reg.public_key_b64);
    const pubRaw = libp2pEd25519Key(pubProto);
    const sig = b64decode(reg.sig_b64);
    if (!pubRaw || sig.length !== 64) return false;
    if (!ed25519Verify(sig, regCanonical(reg.peer_id, reg.public_key_b64, reg.addrs, reg.ts_ms), pubRaw)) {
      return false;
    }
    return peerIdFromLibp2pKey(pubProto) === reg.peer_id;
  } catch {
    return false;
  }
}

/** room: signature verifies AND the host key derives host_peer_id
 *  (reference room.ts: !ok || derivedHost !== hostPeerId -> 403). */
export function roomHostVerified(rec: HubRoomHost): boolean {
  try {
    const pubProto = b64decode(rec.host_public_key_b64);
    const pubRaw = libp2pEd25519Key(pubProto);
    const sig = b64decode(rec.sig_b64);
    if (!pubRaw || sig.length !== 64) return false;
    if (!ed25519Verify(sig, roomCanonical(rec.room_id, rec.host_peer_id, rec.host_public_key_b64, rec.ts_ms), pubRaw)) {
      return false;
    }
    return peerIdFromLibp2pKey(pubProto) === rec.host_peer_id;
  } catch {
    return false;
  }
}

/** inbox item: signature verifies under the included sender key. The reference
 *  inbox.ts does NOT check the key→from binding server-side — the recipient
 *  does that client-side (hub.rs verify_mail_item). env_json stays opaque. */
export function mailItemVerified(item: HubMailItem): boolean {
  try {
    const pubRaw = libp2pEd25519Key(b64decode(item.public_key_b64));
    const sig = b64decode(item.sig_b64);
    if (!pubRaw || sig.length !== 64) return false;
    return ed25519Verify(sig, mailCanonical(item.from, item.to, item.ts_ms, item.env_json), pubRaw);
  } catch {
    return false;
  }
}

/** drain ownership: the drain signature must verify against the peer's
 *  REGISTERED public key (reference inbox/[peerId].ts). No reg record -> false.
 *  A valid signature from a DIFFERENT key fails -> a stranger cannot drain. */
export function drainAuthorized(
  peerId: string,
  tsMs: number,
  sigB64: string,
  registeredPubB64: string | undefined,
): boolean {
  try {
    const pubRaw = registeredPubB64 ? libp2pEd25519Key(b64decode(registeredPubB64)) : null;
    const sig = b64decode(sigB64);
    if (!pubRaw || sig.length !== 64) return false;
    return ed25519Verify(sig, drainCanonical(peerId, tsMs), pubRaw);
  } catch {
    return false;
  }
}
