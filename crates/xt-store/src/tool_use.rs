//! The single owner of structural tool classification. Ingestion derives these
//! fields in memory, before content retention decides whether the block that
//! carried them survives, so metadata-only rows keep name/kind/server/tool/skill.
//!
//! The kind strings are stable and are the ones the schema already accepts:
//! `builtin`, `mcp`, `skill`, `hook`, `command` (a slash command) and
//! `subagent`. Nothing here reads shell command text, tool arguments, prompts or
//! any other free text: a detail that is not explicitly structural stays unknown
//! rather than being guessed at.

use crate::ingest::ToolKind;
use serde_json::Value;

/// `mcp__<server>__<tool>`; the tool part may itself contain the separator.
pub const MCP_PREFIX: &str = "mcp__";
const MCP_SEPARATOR: &str = "__";
/// The builtin whose explicit input names a skill.
pub const SKILL_TOOL: &str = "Skill";
/// Both spellings of the subagent builtin.
pub const SUBAGENT_TOOLS: [&str; 2] = ["Agent", "Task"];
/// The structural name of a stop-hook summary. M-17 counts one such event per
/// summary record, never one per hook the summary happens to describe.
pub const HOOK_EVENT_NAME: &str = "stop_hook_summary";

/// A classified tool identity. `kind` is always known; the details are known
/// only when the name (or an explicit structural input field) states them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Structure {
    pub kind: ToolKind,
    pub server: Option<String>,
    pub tool: Option<String>,
    pub skill: Option<String>,
}

impl Structure {
    fn plain(kind: ToolKind) -> Self {
        Self {
            kind,
            server: None,
            tool: None,
            skill: None,
        }
    }
}

/// Classify one assistant tool call. `input` is the block's own input value when
/// one is present; only explicitly named structural fields are read from it.
pub fn classify(name: &str, input: Option<&Value>) -> Structure {
    let name = name.trim();
    if let Some(detail) = name.strip_prefix(MCP_PREFIX) {
        // A valid detail is a nonempty server, the separator, then a nonempty
        // remainder. Anything else is malformed: the call is still an MCP call,
        // but which server and tool it names stays unknown rather than invented.
        let (server, tool) = detail.split_once(MCP_SEPARATOR).unwrap_or_default();
        let named = !server.is_empty() && !tool.is_empty();
        return Structure {
            kind: ToolKind::Mcp,
            server: named.then(|| server.to_owned()),
            tool: named.then(|| tool.to_owned()),
            skill: None,
        };
    }
    if name == SKILL_TOOL {
        return Structure {
            skill: explicit_skill(input),
            ..Structure::plain(ToolKind::Skill)
        };
    }
    if SUBAGENT_TOOLS.contains(&name) {
        return Structure::plain(ToolKind::Subagent);
    }
    // Every ordinary tool name, including host-specific shells and editors.
    Structure::plain(ToolKind::Builtin)
}

/// Take the skill name the caller stated as a structural field. Arguments are
/// never read, and a missing, blank or non-string field leaves the name unknown.
fn explicit_skill(input: Option<&Value>) -> Option<String> {
    input?
        .get("skill")?
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ordinary_names_are_builtin_and_never_parsed_as_shell_text() {
        for name in [
            "Bash",
            "Shell",
            "exec_command",
            "Edit",
            "Read",
            "mcp",
            "  Edit  ",
        ] {
            assert_eq!(
                classify(
                    name,
                    Some(&json!({"command": "rm -rf /", "skill": "ignored"}))
                ),
                Structure::plain(ToolKind::Builtin),
                "{name}"
            );
        }
    }

    #[test]
    fn mcp_names_split_server_from_the_remaining_tool() {
        assert_eq!(
            classify("mcp__Claude_Browser__computer", None),
            Structure {
                kind: ToolKind::Mcp,
                server: Some("Claude_Browser".into()),
                tool: Some("computer".into()),
                skill: None,
            }
        );
        // The tool part keeps its own separators instead of being split again.
        assert_eq!(
            classify("mcp__server__tool__with__parts", None)
                .tool
                .as_deref(),
            Some("tool__with__parts")
        );
    }

    #[test]
    fn malformed_mcp_detail_stays_unknown() {
        for name in ["mcp__", "mcp__server", "mcp____tool", "mcp__server__"] {
            assert_eq!(
                classify(name, None),
                Structure::plain(ToolKind::Mcp),
                "{name}"
            );
        }
    }

    #[test]
    fn skill_takes_only_the_explicit_name_and_subagents_take_both_spellings() {
        assert_eq!(
            classify(
                "Skill",
                Some(&json!({"skill": "  memhub:login  ", "args": "--status"}))
            )
            .skill,
            Some("memhub:login".into())
        );
        for input in [
            None,
            Some(json!({})),
            Some(json!({"skill": ""})),
            Some(json!({"skill": 7})),
            Some(json!({"args": "login"})),
        ] {
            assert_eq!(
                classify("Skill", input.as_ref()),
                Structure::plain(ToolKind::Skill),
                "{input:?}"
            );
        }
        for name in SUBAGENT_TOOLS {
            assert_eq!(classify(name, None), Structure::plain(ToolKind::Subagent));
        }
    }
}
