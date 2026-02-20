use base64::Engine;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const CONTEXT_DIR: &str = "contexts";
pub const SESSIONS_DIR: &str = "sessions";
pub const DEFAULT_NON_ADMIN_CONTEXT_FILE: &str = "contexts/NON_ADMIN.md";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionTurn {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub sender_id: Option<String>,
    #[serde(default)]
    pub sender_name: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub reply_target: Option<String>,
    #[serde(default)]
    pub timestamp: Option<u64>,
}

pub const OPENCLAW_PROMPT_FILES: [&str; 8] = [
    "AGENTS.md",
    "SOUL.md",
    "TOOLS.md",
    "IDENTITY.md",
    "USER.md",
    "HEARTBEAT.md",
    "BOOTSTRAP.md",
    "MEMORY.md",
];

pub const OPENCLAW_CONTEXT_FILES: [&str; 9] = [
    "IDENTITY.md",
    "AGENTS.md",
    "HEARTBEAT.md",
    "SOUL.md",
    "USER.md",
    "TOOLS.md",
    "BOOTSTRAP.md",
    "MEMORY.md",
    "NON_ADMIN.md",
];

pub fn context_dir_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(CONTEXT_DIR)
}

fn canonical_context_filename(filename: &str) -> Option<&'static str> {
    OPENCLAW_CONTEXT_FILES
        .iter()
        .copied()
        .find(|known| known.eq_ignore_ascii_case(filename))
}

pub fn context_file_relative_path(filename: &str) -> String {
    format!("{CONTEXT_DIR}/{filename}")
}

pub fn preferred_context_file_path(workspace_dir: &Path, filename: &str) -> PathBuf {
    context_dir_path(workspace_dir).join(filename)
}

pub fn legacy_context_file_path(workspace_dir: &Path, filename: &str) -> PathBuf {
    workspace_dir.join(filename)
}

pub fn resolve_context_file_for_read(workspace_dir: &Path, filename: &str) -> PathBuf {
    let preferred = preferred_context_file_path(workspace_dir, filename);
    if preferred.is_file() {
        return preferred;
    }

    let legacy = legacy_context_file_path(workspace_dir, filename);
    if legacy.is_file() {
        return legacy;
    }

    preferred
}

pub fn openclaw_context_file_exists(workspace_dir: &Path, filename: &str) -> bool {
    preferred_context_file_path(workspace_dir, filename).is_file()
        || legacy_context_file_path(workspace_dir, filename).is_file()
}

pub fn ensure_context_dir(workspace_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = context_dir_path(workspace_dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn sessions_dir_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(SESSIONS_DIR)
}

pub fn ensure_sessions_dir(workspace_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = sessions_dir_path(workspace_dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn session_filename(session_id: &str) -> String {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(session_id.as_bytes());
    format!("{encoded}.md")
}

fn decode_session_stem(stem: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(stem.as_bytes())
        .ok()?;
    String::from_utf8(bytes).ok()
}

pub fn preferred_session_file_path(workspace_dir: &Path, session_id: &str) -> PathBuf {
    sessions_dir_path(workspace_dir).join(session_filename(session_id))
}

pub fn legacy_session_file_path(workspace_dir: &Path, session_id: &str) -> PathBuf {
    sessions_dir_path(workspace_dir).join(format!("{session_id}.md"))
}

pub fn resolve_session_file_for_read(workspace_dir: &Path, session_id: &str) -> PathBuf {
    let preferred = preferred_session_file_path(workspace_dir, session_id);
    if preferred.is_file() {
        return preferred;
    }

    let legacy = legacy_session_file_path(workspace_dir, session_id);
    if legacy.is_file() {
        return legacy;
    }

    preferred
}

pub fn append_session_turn(
    workspace_dir: &Path,
    session_id: &str,
    turn: &SessionTurn,
) -> std::io::Result<PathBuf> {
    let sid = session_id.trim();
    if sid.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "session_id cannot be empty",
        ));
    }

    let text = turn.text.trim();
    if text.is_empty() {
        return Ok(resolve_session_file_for_read(workspace_dir, sid));
    }

    let _ = ensure_sessions_dir(workspace_dir)?;
    let preferred = preferred_session_file_path(workspace_dir, sid);
    let legacy = legacy_session_file_path(workspace_dir, sid);
    let target = if preferred.is_file() || !legacy.is_file() {
        preferred
    } else {
        legacy
    };

    let mut normalized = turn.clone();
    normalized.role = normalized.role.trim().to_string();
    if normalized.role.is_empty() {
        normalized.role = "user".to_string();
    }
    normalized.text = text.to_string();

    let serialized = serde_json::to_string(&normalized)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&target)?;

    if file.metadata()?.len() == 0 {
        writeln!(file, "# ZeroClaw Session Transcript")?;
        writeln!(file, "<!-- zeroclaw-session:v1 -->")?;
        writeln!(file, "<!-- session_id: {sid} -->")?;
        writeln!(file)?;
    }

    writeln!(file, "- {serialized}")?;
    Ok(target)
}

fn parse_session_turn_line(line: &str) -> Option<SessionTurn> {
    let trimmed = line.trim();
    let payload = if let Some(rest) = trimmed.strip_prefix("- ") {
        rest.trim()
    } else if trimmed.starts_with('{') {
        trimmed
    } else {
        return None;
    };

    if payload.is_empty() {
        return None;
    }

    let parsed = serde_json::from_str::<SessionTurn>(payload).ok()?;
    if parsed.text.trim().is_empty() {
        return None;
    }
    Some(parsed)
}

pub fn read_session_turns(
    workspace_dir: &Path,
    session_id: &str,
) -> std::io::Result<Vec<SessionTurn>> {
    let path = resolve_session_file_for_read(workspace_dir, session_id);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)?;
    Ok(content
        .lines()
        .filter_map(parse_session_turn_line)
        .collect())
}

pub fn list_session_ids(workspace_dir: &Path) -> std::io::Result<Vec<String>> {
    let dir = sessions_dir_path(workspace_dir);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut session_ids = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if !path.is_file() || path.extension().and_then(|v| v.to_str()) != Some("md") {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|v| v.to_str()) else {
            continue;
        };

        if let Some(decoded) = decode_session_stem(stem) {
            session_ids.push(decoded);
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(found) = content.lines().find_map(|line| {
                line.trim()
                    .strip_prefix("<!-- session_id:")
                    .and_then(|rest| rest.strip_suffix("-->"))
                    .map(str::trim)
                    .filter(|sid| !sid.is_empty())
                    .map(ToOwned::to_owned)
            }) {
                session_ids.push(found);
            }
        }
    }

    session_ids.sort();
    session_ids.dedup();
    Ok(session_ids)
}

pub fn remove_session_file(workspace_dir: &Path, session_id: &str) -> std::io::Result<bool> {
    let mut removed = false;
    let preferred = preferred_session_file_path(workspace_dir, session_id);
    let legacy = legacy_session_file_path(workspace_dir, session_id);

    for path in [preferred, legacy] {
        if path.is_file() {
            std::fs::remove_file(&path)?;
            removed = true;
        }
    }

    Ok(removed)
}

pub fn resolve_non_admin_context_path(workspace_dir: &Path, configured: &str) -> PathBuf {
    let trimmed = configured.trim();
    let relative = if trimmed.is_empty() {
        DEFAULT_NON_ADMIN_CONTEXT_FILE
    } else {
        trimmed
    };

    if Path::new(relative).is_absolute() {
        return PathBuf::from(relative);
    }

    if relative.eq_ignore_ascii_case(DEFAULT_NON_ADMIN_CONTEXT_FILE)
        || relative.eq_ignore_ascii_case("NON_ADMIN.md")
    {
        let preferred = preferred_context_file_path(workspace_dir, "NON_ADMIN.md");
        if preferred.is_file() {
            return preferred;
        }
        let legacy = legacy_context_file_path(workspace_dir, "NON_ADMIN.md");
        if legacy.is_file() {
            return legacy;
        }
    }

    workspace_dir.join(relative)
}

/// For tool calls, transparently map legacy root paths (e.g. `AGENTS.md`)
/// to the new `contexts/` location when applicable.
pub fn map_context_tool_path_alias(workspace_dir: &Path, requested_path: &str) -> String {
    let trimmed = requested_path.trim();
    if trimmed.is_empty() || Path::new(trimmed).is_absolute() {
        return trimmed.to_string();
    }

    let normalized = trimmed.trim_start_matches("./").replace('\\', "/");
    if normalized.contains('/') {
        return trimmed.to_string();
    }

    let Some(canonical) = canonical_context_filename(&normalized) else {
        return trimmed.to_string();
    };

    let preferred = preferred_context_file_path(workspace_dir, canonical);
    let legacy = legacy_context_file_path(workspace_dir, canonical);
    if preferred.is_file() || !legacy.is_file() {
        return context_file_relative_path(canonical);
    }

    trimmed.to_string()
}

pub fn protected_prompt_relative_paths() -> Vec<String> {
    OPENCLAW_CONTEXT_FILES
        .iter()
        .flat_map(|filename| {
            [
                context_file_relative_path(filename),
                (*filename).to_string(),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_context_file_prefers_contexts_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _ = ensure_context_dir(tmp.path()).unwrap();
        std::fs::write(preferred_context_file_path(tmp.path(), "SOUL.md"), "new").unwrap();
        std::fs::write(legacy_context_file_path(tmp.path(), "SOUL.md"), "old").unwrap();

        let resolved = resolve_context_file_for_read(tmp.path(), "SOUL.md");
        assert_eq!(resolved, preferred_context_file_path(tmp.path(), "SOUL.md"));
    }

    #[test]
    fn resolve_context_file_falls_back_to_legacy_root() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(legacy_context_file_path(tmp.path(), "SOUL.md"), "legacy").unwrap();

        let resolved = resolve_context_file_for_read(tmp.path(), "SOUL.md");
        assert_eq!(resolved, legacy_context_file_path(tmp.path(), "SOUL.md"));
    }

    #[test]
    fn resolve_non_admin_context_supports_legacy_default() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            legacy_context_file_path(tmp.path(), "NON_ADMIN.md"),
            "legacy non admin",
        )
        .unwrap();

        let resolved = resolve_non_admin_context_path(tmp.path(), "contexts/NON_ADMIN.md");
        assert_eq!(
            resolved,
            legacy_context_file_path(tmp.path(), "NON_ADMIN.md")
        );
    }

    #[test]
    fn tool_path_alias_maps_legacy_filename_to_contexts_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let _ = ensure_context_dir(tmp.path()).unwrap();
        std::fs::write(preferred_context_file_path(tmp.path(), "AGENTS.md"), "ctx").unwrap();

        let mapped = map_context_tool_path_alias(tmp.path(), "AGENTS.md");
        assert_eq!(mapped, "contexts/AGENTS.md");
    }

    #[test]
    fn tool_path_alias_keeps_legacy_filename_when_only_root_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(legacy_context_file_path(tmp.path(), "AGENTS.md"), "legacy").unwrap();

        let mapped = map_context_tool_path_alias(tmp.path(), "AGENTS.md");
        assert_eq!(mapped, "AGENTS.md");
    }

    #[test]
    fn session_turns_roundtrip_in_markdown_file() {
        let tmp = tempfile::tempdir().unwrap();
        append_session_turn(
            tmp.path(),
            "onebot_v11:group:20001",
            &SessionTurn {
                role: "user".to_string(),
                text: "hello".to_string(),
                sender_id: Some("10001".to_string()),
                sender_name: Some("Alice".to_string()),
                channel: Some("onebot_v11".to_string()),
                reply_target: Some("group:20001".to_string()),
                timestamp: Some(1_700_000_000),
            },
        )
        .unwrap();

        let turns = read_session_turns(tmp.path(), "onebot_v11:group:20001").unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].text, "hello");
        assert_eq!(turns[0].sender_id.as_deref(), Some("10001"));
    }

    #[test]
    fn list_session_ids_reads_encoded_filenames() {
        let tmp = tempfile::tempdir().unwrap();
        append_session_turn(
            tmp.path(),
            "onebot_v11:private:10001",
            &SessionTurn {
                role: "assistant".to_string(),
                text: "hi".to_string(),
                ..SessionTurn::default()
            },
        )
        .unwrap();

        let ids = list_session_ids(tmp.path()).unwrap();
        assert_eq!(ids, vec!["onebot_v11:private:10001".to_string()]);
    }
}
