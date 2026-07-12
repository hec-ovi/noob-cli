//! System prompt assembly. Assembled ONCE per session in a fixed order (the
//! order is a cache invariant); nothing here re-runs per request. Plan mode
//! never touches this head.

use std::path::Path;

pub const BASE_MD: &str = include_str!("../../prompts/base.md");
pub const COMPACT_MD: &str = include_str!("../../prompts/compact.md");

/// AGENTS.md files are user input of unbounded size; each is hard-capped.
const AGENTS_CAP: usize = 16 * 1024;

pub struct PromptInputs {
    pub cwd: String,
    pub model: String,
    /// "container" | "workspace" | "off (--yolo)"
    pub sandbox: String,
    pub global_agents: Option<String>,
    pub project_agents: Option<String>,
    /// One `- name: description` line per skill (P3); None until then.
    pub skills_index: Option<String>,
    /// One line naming configured MCP servers (P4); None until then.
    pub mcp_line: Option<String>,
}

/// Layers 1+2: identity + environment block. <= 560 tokens, budget-tested.
/// The environment block is computed once at session start, never per
/// request: a date that rolled over mid-session would bust the cache prefix.
pub fn head(inputs: &PromptInputs) -> String {
    format!(
        "{BASE_MD}\n<env>\ncwd: {}\nplatform: {}\ndate: {}\nmodel: {}\nsandbox: {}\n</env>",
        inputs.cwd,
        std::env::consts::OS,
        today_utc(),
        inputs.model,
        inputs.sandbox,
    )
}

/// The full system prompt: head + AGENTS.md layers + skills index + MCP line.
pub fn assemble(inputs: &PromptInputs) -> String {
    assemble_from(head(inputs), inputs)
}

/// Assemble on top of an already-computed head, so a caller that needs both
/// (debug prompt) computes the head exactly once: two head() calls straddling
/// midnight would disagree on the date.
pub fn assemble_from(head: String, inputs: &PromptInputs) -> String {
    let mut out = head;
    if let Some(global) = &inputs.global_agents {
        out.push_str("\n\n# Global instructions (AGENTS.md)\n\n");
        push_capped(&mut out, global);
    }
    if let Some(project) = &inputs.project_agents {
        out.push_str("\n\n# Project instructions (AGENTS.md)\n\n");
        push_capped(&mut out, project);
    }
    if let Some(skills) = &inputs.skills_index {
        // Resolver discipline (thin harness, fat skills): this index is the
        // dispatcher; bodies cost zero tokens until a match loads one.
        out.push_str(
            "\n\n# Skills (resolver)\n\nMatch the task against these skills. Load a \
             matching skill with the skill tool and follow it before acting; if two \
             match, load both.\n\n",
        );
        out.push_str(skills);
    }
    if let Some(mcp) = &inputs.mcp_line {
        out.push('\n');
        out.push_str(mcp);
    }
    out
}

fn push_capped(out: &mut String, text: &str) {
    let text = text.trim_end();
    if text.len() <= AGENTS_CAP {
        out.push_str(text);
        return;
    }
    let mut cut = AGENTS_CAP;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    out.push_str(&text[..cut]);
    out.push_str("\n[AGENTS.md truncated at 16 KiB]");
}

/// The one-line MCP layer: server names only; schemas stay out of the head
/// forever (mcp_connect returns catalogs as tool results).
pub fn mcp_line(servers: &[crate::mcp::config::ServerConfig]) -> Option<String> {
    if servers.is_empty() {
        return None;
    }
    let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
    Some(format!("MCP servers (use mcp_connect): {}", names.join(", ")))
}

/// Read one AGENTS.md if present and non-empty.
pub fn load_agents_md(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("AGENTS.md")).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// YYYY-MM-DD in UTC, hand-rolled (no chrono). Days-to-civil per Howard
/// Hinnant's algorithm.
pub fn today_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> PromptInputs {
        PromptInputs {
            cwd: "/work".into(),
            model: "qwen".into(),
            sandbox: "container".into(),
            global_agents: None,
            project_agents: None,
            skills_index: None,
            mcp_line: None,
        }
    }

    #[test]
    fn civil_date_reference_values() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1)); // leap year start
        assert_eq!(civil_from_days(19_782), (2024, 2, 29)); // leap day
        assert_eq!(civil_from_days(20_638), (2026, 7, 4));
    }

    #[test]
    fn head_contains_the_env_block_in_fixed_order() {
        let h = head(&inputs());
        let env_at = h.find("<env>").unwrap();
        let body = &h[env_at..];
        let order = ["cwd: /work", "platform: ", "date: ", "model: qwen", "sandbox: container"];
        let mut at = 0;
        for needle in order {
            let pos = body[at..].find(needle).expect(needle);
            at += pos;
        }
        assert!(body.ends_with("</env>"));
    }

    #[test]
    fn assemble_without_extras_is_exactly_the_head() {
        assert_eq!(assemble(&inputs()), head(&inputs()));
    }

    #[test]
    fn base_prompt_tells_the_agent_to_execute_the_plan_not_propose_it() {
        // Guards the autonomy directive: local models were laying out a plan and
        // waiting for approval, or asking where files are, instead of proceeding.
        let b = BASE_MD.to_lowercase();
        assert!(b.contains("carry it out"), "base.md must tell the agent to execute its plan");
        assert!(
            b.contains("do not stop to ask"),
            "base.md must forbid stopping to ask for plan approval"
        );
        assert!(
            b.contains("never ask the user for something you can find"),
            "base.md must forbid asking for what the agent can discover itself"
        );
    }

    #[test]
    fn agents_md_layers_append_in_order_global_then_project() {
        let mut i = inputs();
        i.global_agents = Some("be global".into());
        i.project_agents = Some("be local".into());
        let s = assemble(&i);
        let g = s.find("# Global instructions (AGENTS.md)").unwrap();
        let p = s.find("# Project instructions (AGENTS.md)").unwrap();
        assert!(g < p);
        assert!(s.contains("be global"));
        assert!(s.contains("be local"));
    }

    #[test]
    fn oversize_agents_md_is_capped_with_a_notice() {
        let mut i = inputs();
        i.global_agents = Some("x".repeat(20 * 1024));
        let s = assemble(&i);
        assert!(s.contains("[AGENTS.md truncated at 16 KiB]"));
        assert!(s.len() < 20 * 1024);
    }

    #[test]
    fn mcp_line_lists_names_only_and_lands_last() {
        use crate::mcp::config::{ServerConfig, TransportConfig};
        let servers = vec![
            ServerConfig {
                name: "fs".into(),
                transport: TransportConfig::Stdio { command: "fs-mcp".into(), args: vec![] },
                timeout: std::time::Duration::from_secs(30),
            },
            ServerConfig {
                name: "websearch".into(),
                transport: TransportConfig::Http { url: "http://localhost:8000".into() },
                timeout: std::time::Duration::from_secs(30),
            },
        ];
        assert_eq!(
            mcp_line(&servers).unwrap(),
            "MCP servers (use mcp_connect): fs, websearch"
        );
        assert!(mcp_line(&[]).is_none());
        let mut i = inputs();
        i.skills_index = Some("- fmt: formats".into());
        i.mcp_line = mcp_line(&servers);
        let s = assemble(&i);
        // The MCP line is the last layer, after the resolver section, and
        // never carries schemas or URLs.
        assert!(s.ends_with("MCP servers (use mcp_connect): fs, websearch"));
        assert!(!s.contains("localhost:8000"));
        assert!(s.find("# Skills (resolver)").unwrap() < s.find("MCP servers").unwrap());
    }

    #[test]
    fn base_prompt_has_no_cap_phrasing() {
        // The full lint lives in the budget e2e; this is the fast guard.
        for banned in ["keep it brief", "in 50 words", "max 3 sentences"] {
            assert!(!BASE_MD.to_lowercase().contains(banned));
        }
    }

    #[test]
    fn load_agents_md_skips_missing_and_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load_agents_md(tmp.path()).is_none());
        std::fs::write(tmp.path().join("AGENTS.md"), "  \n").unwrap();
        assert!(load_agents_md(tmp.path()).is_none());
        std::fs::write(tmp.path().join("AGENTS.md"), "rule\n").unwrap();
        assert_eq!(load_agents_md(tmp.path()).unwrap(), "rule");
    }
}
