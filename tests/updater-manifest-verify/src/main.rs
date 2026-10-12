//! Release-train rehearsal: verify the STAGED updater manifest exactly
//! the way the shipped client verifies it.
//!
//! Run from the repo root after staging a release:
//!
//! ```text
//! cargo run --manifest-path tests/updater-manifest-verify/Cargo.toml
//! ```
//!
//! It replays the tauri-plugin-updater 2.13.3 chain over LOCAL bytes
//! (no network): the public key is the one embedded at build time in
//! `crates/modelswarm-desktop/tauri.conf.json` (`plugins.updater.pubkey`,
//! base64 of the minisign .pub file — decoded with
//! `base64::engine::general_purpose::STANDARD` then
//! `PublicKey::decode`, mirroring the plugin's `base64_to_string` +
//! `PublicKey::decode`); each `platforms[...]` entry's `signature` is
//! base64 of the whole `.sig` file (`Signature::decode`); verification
//! is `public_key.verify(bytes, &signature, /* allow_legacy */ true)`
//! over the STAGED artifact bytes; and `requireSignedVersion: true` is
//! emulated by requiring the signature-covered trusted comment to carry
//! `version:<manifest version>` (tab-separated field, v-prefix trimmed,
//! literal comparison — the plugin's fallback path).
//!
//! Exit code 0 only if every platform entry verifies AND the
//! `/api/download/<file>` targets are shaped like the counting route
//! accepts (`^[A-Za-z0-9][A-Za-z0-9._-]*\.(exe|deb|AppImage|dmg)$`,
//! apps/tracker/app/api/download/[file]/route.ts NAME_PATTERN) and
//! exist on disk in the staging directory.

use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

const ORIGIN: &str = "https://modelswarm.deepflux.space";
const ROUTE: &str = "/api/download/";
const NAME_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9._-]*\\.(exe|deb|AppImage|dmg)$";

fn fail(context: &str, message: &str) {
    println!("FAIL [{context}] {message}");
}

fn main() -> ExitCode {
    let root = PathBuf::from(".").canonicalize().expect("repo root");
    let downloads = root.join("apps/tracker/public/downloads");
    let manifest_path = downloads.join("latest.json");
    let conf_path = root.join("crates/modelswarm-desktop/tauri.conf.json");

    let manifest_raw = match fs::read_to_string(&manifest_path) {
        Ok(raw) => raw,
        Err(err) => {
            fail(
                "manifest",
                &format!("cannot read {}: {err}", manifest_path.display()),
            );
            return ExitCode::FAILURE;
        }
    };
    let manifest: serde_json::Value = match serde_json::from_str(&manifest_raw) {
        Ok(value) => value,
        Err(err) => {
            fail("manifest", &format!("latest.json is not valid JSON: {err}"));
            return ExitCode::FAILURE;
        }
    };
    println!(
        "PASS [manifest] latest.json parses as JSON ({})",
        manifest_path.display()
    );

    let version = manifest["version"].as_str().unwrap_or_default().to_string();
    if version.is_empty() {
        fail("manifest", "top-level version missing/empty");
        return ExitCode::FAILURE;
    }
    let pub_date = manifest["pub_date"].as_str().unwrap_or_default();
    // RFC 3339 shape the plugin's OffsetDateTime parser accepts; strict
    // enough to catch a stale or hand-mangled date.
    let date_ok = pub_date.len() >= 20
        && pub_date.as_bytes()[4] == b'-'
        && pub_date.as_bytes()[10] == b'T'
        && pub_date.ends_with('Z');
    println!(
        "PASS [manifest] version={version} pub_date={pub_date} notes={} chars",
        manifest["notes"].as_str().map(str::len).unwrap_or(0)
    );
    if !date_ok {
        fail("manifest", "pub_date is missing or not RFC 3339 UTC");
        return ExitCode::FAILURE;
    }
    if manifest["notes"]
        .as_str()
        .map(str::is_empty)
        .unwrap_or(true)
    {
        fail("manifest", "notes missing/empty (honest-notes rule)");
        return ExitCode::FAILURE;
    }

    // The embedded key, decoded exactly as the plugin decodes it.
    let conf: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&conf_path).expect("tauri.conf.json"))
            .expect("tauri.conf.json JSON");
    let pubkey_b64 = conf["plugins"]["updater"]["pubkey"]
        .as_str()
        .unwrap_or_default();
    let pubkey_file = base64::engine::general_purpose::STANDARD
        .decode(pubkey_b64)
        .and_then(|raw| {
            String::from_utf8(raw).map_err(|_| base64::DecodeError::InvalidLastSymbol(0, 0))
        })
        .unwrap_or_default();
    let public_key = match PublicKey::decode(&pubkey_file) {
        Ok(key) => key,
        Err(err) => {
            fail(
                "pubkey",
                &format!("embedded pubkey does not decode: {err:?}"),
            );
            return ExitCode::FAILURE;
        }
    };
    println!("PASS [pubkey] embedded tauri.conf.json pubkey decodes (minisign)");

    let platforms = match manifest["platforms"].as_object() {
        Some(map) if !map.is_empty() => map,
        _ => {
            fail("manifest", "platforms object missing/empty");
            return ExitCode::FAILURE;
        }
    };

    let mut all_ok = true;
    for (platform, entry) in platforms {
        let sig_field = entry["signature"].as_str().unwrap_or_default();
        let url = entry["url"].as_str().unwrap_or_default();
        let ctx = format!("platform {platform}");
        if sig_field.is_empty() || url.is_empty() {
            fail(&ctx, "signature or url missing");
            all_ok = false;
            continue;
        }
        if !url.starts_with(&format!("{ORIGIN}{ROUTE}")) {
            fail(&ctx, &format!("url is not {ORIGIN}{ROUTE}<file>: {url}"));
            all_ok = false;
            continue;
        }
        let file = &url[format!("{ORIGIN}{ROUTE}").len()..];
        // Same strict shape the counting route enforces before it will
        // serve a file (route.ts NAME_PATTERN).
        let first = file.chars().next().unwrap_or('/');
        let shape_ok = file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            && first.is_ascii_alphanumeric()
            && (file.ends_with(".exe")
                || file.ends_with(".deb")
                || file.ends_with(".AppImage")
                || file.ends_with(".dmg"))
            && !file.contains("..");
        if !shape_ok {
            fail(
                &ctx,
                &format!("filename fails NAME_PATTERN {NAME_PATTERN}: {file}"),
            );
            all_ok = false;
            continue;
        }
        let staged = downloads.join(file);
        let bytes = match fs::read(&staged) {
            Ok(bytes) => bytes,
            Err(err) => {
                fail(
                    &ctx,
                    &format!("staged artifact missing ({}): {err}", staged.display()),
                );
                all_ok = false;
                continue;
            }
        };
        // The plugin chain: base64 -> whole-.sig text -> Signature::decode.
        let sig_file = match base64::engine::general_purpose::STANDARD
            .decode(sig_field)
            .and_then(|raw| {
                String::from_utf8(raw).map_err(|_| base64::DecodeError::InvalidLastSymbol(0, 0))
            }) {
            Ok(text) => text,
            Err(err) => {
                fail(
                    &ctx,
                    &format!("signature field is not base64 of a .sig file: {err}"),
                );
                all_ok = false;
                continue;
            }
        };
        let signature = match Signature::decode(&sig_file) {
            Ok(sig) => sig,
            Err(err) => {
                fail(
                    &ctx,
                    &format!("signature does not decode as minisign: {err:?}"),
                );
                all_ok = false;
                continue;
            }
        };
        // allow_legacy = true: exactly what the plugin passes, and the
        // only mode in which non-prehashed (Ed) tauri signatures verify.
        if let Err(err) = public_key.verify(&bytes, &signature, true) {
            fail(
                &ctx,
                &format!("MINISIGN VERIFY FAILED over staged bytes of {file}: {err:?}"),
            );
            all_ok = false;
            continue;
        }
        // requireSignedVersion: the trusted comment is only trustworthy
        // after verify() covered it — same order as the plugin.
        let signed_version = signature
            .trusted_comment()
            .split('\t')
            .find_map(|field| field.strip_prefix("version:"));
        match signed_version {
            Some(v) if v.trim_start_matches('v') == version.trim_start_matches('v') => {}
            Some(v) => {
                fail(
                    &ctx,
                    &format!("signed version {v:?} != manifest version {version:?}"),
                );
                all_ok = false;
                continue;
            }
            None => {
                fail(
                    &ctx,
                    "trusted comment carries no version: field (MissingSignedVersion)",
                );
                all_ok = false;
                continue;
            }
        }
        println!(
            "PASS [{ctx}] minisign verify OK over {file} ({n} bytes); signed version == {version}",
            n = bytes.len()
        );
        let digest = sha256_hex(&bytes);
        println!("PASS [{ctx}] sha256({file}) = {digest}");
    }

    if all_ok {
        println!("ALL MANIFEST SIGNATURES VERIFY against the embedded public key over the staged artifact bytes.");
        ExitCode::SUCCESS
    } else {
        println!("MANIFEST VERIFICATION FAILED — do not deploy this staging.");
        ExitCode::FAILURE
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    // Minimal SHA-256 (FIPS 180-4) so this tool needs no hash crate.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bitlen = (bytes.len() as u64) * 8;
    let mut data = bytes.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in data.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    h.iter()
        .map(|word| format!("{word:08x}"))
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::sha256_hex;

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // > one 64-byte block, exercises the padding branches.
        assert_eq!(
            sha256_hex(&[b'a'; 1000]),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }
}
