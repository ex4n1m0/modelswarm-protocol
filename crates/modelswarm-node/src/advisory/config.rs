//! `advisory.json` — the advisory plane's dedicated local config file
//! (ADR-033 §4, TR-RC-8) — and the key-file resolution (ADR-033 §3.4).
//!
//! TR-RC-8's ruling, honored structurally: the file is
//! `<data_dir>/advisory.json`, NOT `config.json` — the desktop's
//! `config.json` writer rewrites that file wholesale and unknown keys
//! must not live where a two-field writer can delete them. This module
//! READS `advisory.json` and never writes it.
//!
//! The JSON form is one flat object whose keys are the ADR-033 §4
//! table's dotted suffixes (`enabled`, `endpoint_url`, …); unknown keys
//! are tolerated on read. The nine ADR-037 §3.4 `assistant.*` rows are
//! read at this leg but INERT — the assistant surface is a later
//! implementation leg (ADR-037 §7 step 3); reading them now prepares
//! that leg without enabling anything.
//!
//! No rotation anywhere (ADR-037 §3.6 standing owner directive): this
//! module never writes, moves, or regenerates key material.

use std::path::{Path, PathBuf};

/// Advisory config file name under the data dir (ADR-033 §4, TR-RC-8).
pub const ADVISORY_CONFIG_FILE: &str = "advisory.json";
/// Env var naming the advisory key file (env-first resolution, §3.4).
pub const ENV_ADVISORY_KEY_FILE: &str = "MSP_ADVISORY_KEY_FILE";

/// Advisory-plane configuration. Digest-plane caps per ADR-033 §4
/// (frozen defaults, tunable without an ADR); the `assistant_*` block is
/// the ADR-037 §3.4 nine-row set — READ-BUT-INERT at this leg.
#[derive(Debug, Clone, PartialEq)]
pub struct AdvisoryConfig {
    /// Master switch; `false` (the default) or a missing key file = the
    /// plane is inert by construction (ADR-033 §3.1).
    pub enabled: bool,
    /// Provider endpoint, recorded at setup from the provider console.
    /// `None` = inert (gap `no_config`). Must be `https://` (§3.3).
    pub endpoint_url: Option<String>,
    /// Provider model id, recorded at setup. `None` = inert.
    pub model: Option<String>,
    /// Key file path override; `None` = platform default (§3.4).
    pub key_file: Option<PathBuf>,
    /// Hard daily cap on digest CALLS INITIATED (failures included),
    /// UTC calendar days. Default 24.
    pub max_calls_per_day: u32,
    /// Minimum spacing between calls, ms. Default 900 000 (15 min).
    pub min_interval_ms: u64,
    /// New telemetry events required to trigger a cycle. Default 200.
    pub events_threshold: u64,
    /// Per-call deadline, ms. Default 30 000.
    pub request_timeout_ms: u64,
    /// Request body cap, bytes. Default 16 384.
    pub max_request_bytes: usize,
    /// Response body cap, bytes. Default 65 536.
    pub max_response_bytes: usize,
    /// Provider-side output bound per digest call. Default 2 048.
    pub max_completion_tokens: u32,

    /// ADR-037 §3.4 — READ-BUT-INERT at this leg (the assistant surface
    /// is §7 step 3; these values are parsed and golden-pinned only).
    pub assistant: AssistantCaps,
}

/// The nine ADR-037 §3.4 `assistant.*` rows. Inert until the assistant
/// leg lands; carried so the config reader and the golden default pin
/// (§9.2(8) as extended by ADR-037 §4.4) cover the full table.
#[derive(Debug, Clone, PartialEq)]
pub struct AssistantCaps {
    pub enabled: bool,
    pub max_calls_per_day: u32,
    pub max_calls_per_hour: u32,
    pub max_turns_per_conversation: u32,
    pub idle_reset_ms: u64,
    pub history_turns_sent: u32,
    pub max_request_bytes: usize,
    pub max_completion_tokens: u32,
    pub request_timeout_ms: u64,
}

impl Default for AssistantCaps {
    fn default() -> Self {
        Self {
            enabled: false,
            max_calls_per_day: 300,
            max_calls_per_hour: 60,
            max_turns_per_conversation: 40,
            idle_reset_ms: 900_000,
            history_turns_sent: 12,
            max_request_bytes: 49_152,
            max_completion_tokens: 1024,
            request_timeout_ms: 45_000,
        }
    }
}

impl Default for AdvisoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint_url: None,
            model: None,
            key_file: None,
            max_calls_per_day: 24,
            min_interval_ms: 900_000,
            events_threshold: 200,
            request_timeout_ms: 30_000,
            max_request_bytes: 16_384,
            max_response_bytes: 65_536,
            max_completion_tokens: 2_048,
            assistant: AssistantCaps::default(),
        }
    }
}

/// Failures of reading `advisory.json`. A missing file is NOT an error —
/// it is the shipped default (inert) configuration.
#[derive(Debug, thiserror::Error)]
#[error("advisory config: {0}")]
pub struct ConfigError(String);

impl AdvisoryConfig {
    /// Reads `<data_dir>/advisory.json`. Missing file = defaults (the
    /// plane is inert). A present-but-malformed file is an error the
    /// caller surfaces as an inert plane with gap `no_config` — never a
    /// guess (fail closed, ADR-033 §4 malformed handling).
    pub fn load(data_dir: &Path) -> Result<Self, ConfigError> {
        let path = data_dir.join(ADVISORY_CONFIG_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                Self::parse(&text).map_err(|e| ConfigError(format!("{}: {e}", path.display())))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ConfigError(format!("{}: {e}", path.display()))),
        }
    }

    /// Parses the flat-object JSON form (unit-testable core; named
    /// `parse`, not `from_str`, to keep the lint surface honest about
    /// not being the `FromStr` trait).
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let root: serde_json::Value =
            serde_json::from_str(text).map_err(|e| ConfigError(format!("invalid JSON: {e}")))?;
        Self::from_json(&root)
    }

    /// Applies the known keys over the defaults; unknown keys tolerated.
    pub fn from_json(root: &serde_json::Value) -> Result<Self, ConfigError> {
        let obj = root
            .as_object()
            .ok_or_else(|| ConfigError("top level must be a flat JSON object".to_string()))?;
        let mut cfg = Self::default();
        let get_bool = |key: &str| -> Result<Option<bool>, ConfigError> {
            match obj.get(key) {
                None => Ok(None),
                Some(v) => v
                    .as_bool()
                    .map(Some)
                    .ok_or_else(|| ConfigError(format!("{key} must be a boolean"))),
            }
        };
        let get_string = |key: &str| -> Result<Option<String>, ConfigError> {
            match obj.get(key) {
                None => Ok(None),
                Some(v) => v
                    .as_str()
                    .map(|s| s.to_string())
                    .map(Some)
                    .ok_or_else(|| ConfigError(format!("{key} must be a string"))),
            }
        };
        let get_u64 = |key: &str| -> Result<Option<u64>, ConfigError> {
            match obj.get(key) {
                None => Ok(None),
                Some(v) => v
                    .as_u64()
                    .map(Some)
                    .ok_or_else(|| ConfigError(format!("{key} must be an unsigned integer"))),
            }
        };

        if let Some(v) = get_bool("enabled")? {
            cfg.enabled = v;
        }
        if let Some(v) = get_string("endpoint_url")? {
            cfg.endpoint_url = Some(v);
        }
        if let Some(v) = get_string("model")? {
            cfg.model = Some(v);
        }
        if let Some(v) = get_string("key_file")? {
            cfg.key_file = Some(PathBuf::from(v));
        }
        if let Some(v) = get_u64("max_calls_per_day")? {
            cfg.max_calls_per_day = u32::try_from(v)
                .map_err(|_| ConfigError("max_calls_per_day overflows u32".into()))?;
        }
        if let Some(v) = get_u64("min_interval_ms")? {
            cfg.min_interval_ms = v;
        }
        if let Some(v) = get_u64("events_threshold")? {
            cfg.events_threshold = v;
        }
        if let Some(v) = get_u64("request_timeout_ms")? {
            cfg.request_timeout_ms = v;
        }
        if let Some(v) = get_u64("max_request_bytes")? {
            cfg.max_request_bytes = usize::try_from(v)
                .map_err(|_| ConfigError("max_request_bytes overflows usize".into()))?;
        }
        if let Some(v) = get_u64("max_response_bytes")? {
            cfg.max_response_bytes = usize::try_from(v)
                .map_err(|_| ConfigError("max_response_bytes overflows usize".into()))?;
        }
        if let Some(v) = get_u64("max_completion_tokens")? {
            cfg.max_completion_tokens = u32::try_from(v)
                .map_err(|_| ConfigError("max_completion_tokens overflows u32".into()))?;
        }

        // The nine ADR-037 §3.4 rows — flat `assistant.*` keys, parsed
        // and carried, INERT at this leg.
        let a = &mut cfg.assistant;
        if let Some(v) = get_bool("assistant.enabled")? {
            a.enabled = v;
        }
        if let Some(v) = get_u64("assistant.max_calls_per_day")? {
            a.max_calls_per_day = u32::try_from(v)
                .map_err(|_| ConfigError("assistant.max_calls_per_day overflows u32".into()))?;
        }
        if let Some(v) = get_u64("assistant.max_calls_per_hour")? {
            a.max_calls_per_hour = u32::try_from(v)
                .map_err(|_| ConfigError("assistant.max_calls_per_hour overflows u32".into()))?;
        }
        if let Some(v) = get_u64("assistant.max_turns_per_conversation")? {
            a.max_turns_per_conversation = u32::try_from(v).map_err(|_| {
                ConfigError("assistant.max_turns_per_conversation overflows u32".into())
            })?;
        }
        if let Some(v) = get_u64("assistant.idle_reset_ms")? {
            a.idle_reset_ms = v;
        }
        if let Some(v) = get_u64("assistant.history_turns_sent")? {
            a.history_turns_sent = u32::try_from(v)
                .map_err(|_| ConfigError("assistant.history_turns_sent overflows u32".into()))?;
        }
        if let Some(v) = get_u64("assistant.max_request_bytes")? {
            a.max_request_bytes = usize::try_from(v)
                .map_err(|_| ConfigError("assistant.max_request_bytes overflows usize".into()))?;
        }
        if let Some(v) = get_u64("assistant.max_completion_tokens")? {
            a.max_completion_tokens = u32::try_from(v)
                .map_err(|_| ConfigError("assistant.max_completion_tokens overflows u32".into()))?;
        }
        if let Some(v) = get_u64("assistant.request_timeout_ms")? {
            a.request_timeout_ms = v;
        }
        Ok(cfg)
    }
}

/// ADR-033 §3.3 transport pinning, config side: the client rejects any
/// `endpoint_url` that is not `https://` (gap `no_config`).
pub fn is_https_endpoint(url: &str) -> bool {
    let rest = url.strip_prefix("https://").unwrap_or("");
    !rest.is_empty()
        && !rest.starts_with('/')
        && !url.chars().any(char::is_whitespace)
        && !rest.contains('@')
}

/// The platform-default key file path (ADR-033 §3.4):
/// `%USERPROFILE%\ModelSwarm-advisory-key.txt` on Windows,
/// `~/.modelswarm/advisory-key.txt` elsewhere. NEVER inside the repo.
pub fn default_key_file() -> PathBuf {
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(profile).join("ModelSwarm-advisory-key.txt");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".modelswarm")
            .join("advisory-key.txt");
    }
    PathBuf::from(".modelswarm").join("advisory-key.txt")
}

/// Env-first key-file resolution (§3.4): `MSP_ADVISORY_KEY_FILE`, then
/// the config `key_file`, then the platform default.
pub fn resolve_key_file(config: &AdvisoryConfig) -> PathBuf {
    if let Some(path) = std::env::var_os(ENV_ADVISORY_KEY_FILE) {
        return PathBuf::from(path);
    }
    if let Some(path) = &config.key_file {
        return path.clone();
    }
    default_key_file()
}

/// Reads the key: single line, trimmed. `Ok(None)` = no file (the plane
/// stays inert, gap `no_key`). A file with an empty first line is also
/// `None` — never a guess. The key never enters the repo, telemetry
/// (the Redactor `sk-` scrub), or any request payload (the allowlist
/// has no key-bearing field type).
pub fn read_key(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let first = text.lines().next().unwrap_or("").trim().to_string();
            Ok(if first.is_empty() { None } else { Some(first) })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_defaults_pin_adr033_table_and_adr037_assistant_rows() {
        // §9.2(8) as extended by ADR-037 §4.4: the eleven ADR-033 §4
        // defaults PLUS the nine ADR-037 §3.4 defaults, pinned — any
        // value change without an amendment is a red build.
        let cfg = AdvisoryConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.endpoint_url, None);
        assert_eq!(cfg.model, None);
        assert_eq!(cfg.key_file, None);
        assert_eq!(cfg.max_calls_per_day, 24);
        assert_eq!(cfg.min_interval_ms, 900_000);
        assert_eq!(cfg.events_threshold, 200);
        assert_eq!(cfg.request_timeout_ms, 30_000);
        assert_eq!(cfg.max_request_bytes, 16_384);
        assert_eq!(cfg.max_response_bytes, 65_536);
        assert_eq!(cfg.max_completion_tokens, 2_048);
        // ADR-037 §3.4 (read-but-inert at this leg).
        let a = &cfg.assistant;
        assert!(!a.enabled);
        assert_eq!(a.max_calls_per_day, 300);
        assert_eq!(a.max_calls_per_hour, 60);
        assert_eq!(a.max_turns_per_conversation, 40);
        assert_eq!(a.idle_reset_ms, 900_000);
        assert_eq!(a.history_turns_sent, 12);
        assert_eq!(a.max_request_bytes, 49_152);
        assert_eq!(a.max_completion_tokens, 1024);
        assert_eq!(a.request_timeout_ms, 45_000);
    }

    #[test]
    fn missing_file_is_default_inert_config() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = AdvisoryConfig::load(dir.path()).unwrap();
        assert_eq!(cfg, AdvisoryConfig::default());
    }

    #[test]
    fn flat_keys_and_unknown_keys_tolerated() {
        let cfg = AdvisoryConfig::parse(
            r#"{"enabled":true,"endpoint_url":"https://api.example.test/v1/chat/completions",
                "model":"DeepSeek-V4.1-Flash","max_calls_per_day":12,
                "assistant.enabled":true,"assistant.max_calls_per_hour":30,
                "future_key":"whatever"}"#,
        )
        .unwrap();
        assert!(cfg.enabled);
        assert_eq!(
            cfg.endpoint_url.as_deref(),
            Some("https://api.example.test/v1/chat/completions")
        );
        assert_eq!(cfg.model.as_deref(), Some("DeepSeek-V4.1-Flash"));
        assert_eq!(cfg.max_calls_per_day, 12);
        // Assistant rows parse (inert consumers at this leg, values kept).
        assert!(cfg.assistant.enabled);
        assert_eq!(cfg.assistant.max_calls_per_hour, 30);
        assert_eq!(cfg.assistant.max_calls_per_day, 300, "unwritten = default");
    }

    #[test]
    fn malformed_config_fails_closed() {
        assert!(AdvisoryConfig::parse("{nope").is_err());
        assert!(AdvisoryConfig::parse("[1,2]").is_err());
        assert!(AdvisoryConfig::parse(r#"{"enabled":"yes"}"#).is_err());
        assert!(AdvisoryConfig::parse(r#"{"max_calls_per_day":-1}""#).is_err());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(ADVISORY_CONFIG_FILE), "{bad").unwrap();
        assert!(AdvisoryConfig::load(dir.path()).is_err());
    }

    #[test]
    fn https_only_endpoint_gate() {
        assert!(is_https_endpoint("https://api.deepseek.example/chat"));
        assert!(!is_https_endpoint("http://api.deepseek.example/chat"));
        assert!(!is_https_endpoint("https://"));
        assert!(!is_https_endpoint("https://user@host/"));
        assert!(!is_https_endpoint("https://ho st/"));
        assert!(!is_https_endpoint("ftp://x"));
        assert!(!is_https_endpoint(""));
    }

    #[test]
    fn key_file_reads_single_trimmed_line() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("k.txt");
        std::fs::write(&key, "  test-key-local-double  \nsecond line ignored\n").unwrap();
        assert_eq!(
            read_key(&key).unwrap().as_deref(),
            Some("test-key-local-double")
        );
        let absent = dir.path().join("absent.txt");
        assert_eq!(read_key(&absent).unwrap(), None);
        std::fs::write(&key, "   \n").unwrap();
        assert_eq!(read_key(&key).unwrap(), None, "blank first line = no key");
    }

    #[test]
    fn key_resolution_order_env_config_default() {
        // The whole test rides the advisory env lock (A-4): while other
        // tests hold the lock with the env var set, even the LOCK-FREE
        // assertions below must not run (resolve_key_file consults the
        // process env).
        let _env = super::super::env_lock();
        let cfg = AdvisoryConfig::default();
        // Config override wins over platform default.
        let with_cfg = AdvisoryConfig {
            key_file: Some(PathBuf::from("/tmp/owner-key.txt")),
            ..cfg.clone()
        };
        assert_eq!(
            resolve_key_file(&with_cfg),
            PathBuf::from("/tmp/owner-key.txt")
        );
        assert_eq!(resolve_key_file(&cfg), default_key_file());
        // Env wins over both when set.
        std::env::set_var(ENV_ADVISORY_KEY_FILE, "/env-first.txt");
        assert_eq!(resolve_key_file(&with_cfg), PathBuf::from("/env-first.txt"));
        std::env::remove_var(ENV_ADVISORY_KEY_FILE);
    }
}
