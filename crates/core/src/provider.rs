//! Stable agent provider identity (`Start.md` §9.1).
//!
//! A `ProviderId` names one supported agent product (workbuddy, claude-code,
//! codex, antigravity, pi). It is the join key across installations, sessions and resources,
//! so it is a newtype over `String` with explicit construction — never a
//! free-form string flowing through the system.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable identifier of a supported agent provider.
///
/// Well-known ids are provided as constants; providers are registered
/// statically (red line #10 — no dynamic plugin system), so the set of
/// valid ids is closed at compile time in practice.
///
/// Both `parse` and `Deserialize` validate against that closed set — the
/// invariant must hold on the wire/IPC path too, not just in-process
/// construction (fail closed, §3.5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct ProviderId(pub String);

impl ProviderId {
    /// Anthropic Claude Code.
    ///
    /// One `ProviderId` for the CLI and its Desktop counterpart
    /// (`docs/providers/claude-desktop.md`). They share session identity but
    /// write to different roots, so they are exposed as two
    /// `AgentInstallation`s under the same id:
    /// - `claude-code:cli` — `%USERPROFILE%\.claude\`
    /// - `claude-code:desktop` — `%LOCALAPPDATA%\Claude-3p\` (out-of-scope
    ///   for v0.1 cleanup but discovered during detect; reported as a
    ///   second read-only installation rather than its own provider, so the
    ///   closed `ProviderId` set stays intact).
    pub const CLAUDE_CODE: &str = "claude-code";
    /// OpenAI Codex.
    ///
    /// CLI and Desktop share the same state root (`~/.codex/`); they are one
    /// `AgentInstallation` whose `data_roots` include both the state root
    /// and the default workspace root (`~/Documents/Codex/`). Per-session
    /// originator metadata (`state_5.sqlite.threads.thread_source` /
    /// `originator`) is what tells CLI vs Desktop sessions apart inside the
    /// Provider.
    pub const CODEX: &str = "codex";
    /// Pi coding agent, verified against local v3 session headers.
    pub const PI: &str = "pi";
    /// Antigravity shared local conversation state (macOS investigation).
    pub const ANTIGRAVITY: &str = "antigravity";

    /// WorkBuddy desktop app (no separate CLI).
    pub const WORKBUDDY: &str = "workbuddy";

    /// Construct from a known-good literal (e.g. a constant above).
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Construct and validate against the statically known provider set.
    ///
    /// Unknown ids are rejected because they indicate a mismatch between
    /// registry and caller — fail closed (§3.5).
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            Self::CLAUDE_CODE | Self::CODEX | Self::WORKBUDDY | Self::PI | Self::ANTIGRAVITY => {
                Some(Self(id.to_owned()))
            }
            _ => None,
        }
    }

    /// Raw string form (serialization stays newtype-transparent;
    /// deserialization validates, see the manual impl below).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// Manual Deserialize (instead of a derive) so the closed-set invariant holds
// on the wire too: a derived impl would happily round-trip any string.
impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        ProviderId::parse(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown provider id: {raw:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_known_providers() {
        for id in [ProviderId::PI, ProviderId::ANTIGRAVITY] {
            assert_eq!(ProviderId::parse(id), Some(ProviderId::new(id)));
            assert_eq!(
                serde_json::from_str::<ProviderId>(&format!("\"{id}\"")).unwrap(),
                ProviderId::new(id)
            );
        }
        assert_eq!(
            ProviderId::parse(ProviderId::CLAUDE_CODE),
            Some(ProviderId::new("claude-code"))
        );
        assert_eq!(
            ProviderId::parse(ProviderId::CODEX),
            Some(ProviderId::new("codex"))
        );
        assert_eq!(
            ProviderId::parse(ProviderId::WORKBUDDY),
            Some(ProviderId::new("workbuddy"))
        );
    }

    #[test]
    fn parse_rejects_unknown_providers() {
        assert!(ProviderId::parse("chatgpt").is_none());
        assert!(ProviderId::parse("").is_none());
    }

    #[test]
    fn deserialize_rejects_unknown_providers() {
        // The closed-set invariant must hold on the wire path too —
        // deserializing an id that parse() rejects is an error, not a
        // round-tripped ProviderId (fail closed, §3.5).
        assert!(serde_json::from_str::<ProviderId>("\"chatgpt\"").is_err());
        assert!(serde_json::from_str::<ProviderId>("\"\"").is_err());
        let ok: ProviderId = serde_json::from_str("\"codex\"").unwrap();
        assert_eq!(ok.as_str(), "codex");
    }
}
