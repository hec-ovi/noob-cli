//! Token-budget enforcement on the SHIPPED artifact: `noob debug prompt
//! --json` prints the exact system prompt and wire tools array the binary
//! sends; tiktoken o200k tokenizes them against the locked ceilings.
//! The config dir here is empty, so the measured prompt is the embedded
//! defaults (agents-default.md + tools-default.md): these ceilings guard
//! what ships, not whatever a user puts in AGENTS.md or TOOLS.md.
//! The live suite closes the loop against the real qwen tokenizer via
//! llama-server /tokenize (P7).

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use serde_json::Value;

// The locked budget, in one place so raising it is a visible diff
// (ARCHITECTURE.md, System prompt).
//
// The owner's hard limit for the whole fixed prefix is 2,000 tokens. These
// ceilings sit below it on purpose: they are drift guards, not the budget.
// Raise one only for a change that earns it, and never past OWNER_HARD_LIMIT.
//
// Note what this cost actually is. The fixed prefix is prefilled once per
// session and then served from the provider's prompt cache; a 16-request run
// in the local corpus computed 517 tokens on the first call and 23 to 844 on
// each later one against a transcript that grew to 14,699. So these tokens
// are a one-time compute plus a permanent slice of the context window, not a
// per-request charge. What does re-cost them is anything that rewrites the
// prefix mid-session, which is why the environment block is built once and
// why compaction is the single sanctioned prefix break.
// These count with tiktoken, which no served model here uses. They are a drift
// alarm, not the real budget: measured against the live endpoint on 2026-07-28,
// the same fixed prefix with every tool registered is 2,335 tokens through
// Laguna S 2.1's tokenizer and about 1,938 through qwen's. A "2,000 token
// prompt" is therefore a statement about a tokenizer, not about this prompt,
// and only the offline number below is comparable run to run.
//
// Raise a ceiling only alongside a capability that pays for itself, and say
// which one in the commit. One was raised for symbol-addressed read and edit
// and then given back: the model never used them, so the cost was permanent
// and the benefit was zero. Harness capability does not belong in the agent's
// prompt; it belongs in the harness.
// TOTAL_CEILING carries the owner-mandated clause in the default TOOLS text
// that names the tool set as the basic one and the file as the user's to
// adjust; that clause is what the 1,925 pays for.
const HEAD_CEILING: usize = 560; // agents + tools defaults + environment block
const TOOLS_CEILING: usize = 1350; // serialized wire tools array
const TOTAL_CEILING: usize = 1925; // total fixed first-request overhead
const OWNER_HARD_LIMIT: usize = 2000; // never exceed, whatever the above say

/// The switches plant one skill, one configured MCP server, and one executable
/// websearch stub, so the full artifact can carry every conditional tool. The
/// ceilings must hold with everything registered, not just the bare core.
fn debug_prompt(with_skill: bool, with_mcp: bool, with_websearch: bool) -> Value {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(config.path().join(".env"), "NOOB_MODEL=qwen3.6-35b-a3b\n").unwrap();
    if with_skill {
        let dir = work.path().join(".noob/skills/budget-probe");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: budget-probe\ndescription: a probe skill for the budget test\n---\nbody\n",
        )
        .unwrap();
    }
    if with_mcp {
        std::fs::write(
            config.path().join("mcp.json"),
            r#"{"servers": {"websearch": {"url": "http://localhost:8000"}}}"#,
        )
        .unwrap();
    }
    let websearch = work.path().join("websearch");
    if with_websearch {
        std::fs::write(&websearch, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&websearch, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noob"));
    // Scrub every NOOB_* variable the binary reads: a host-exported setting
    // (NOOB_SKILL_PATHS, NOOB_CTX, ...) would change the artifact under test.
    noob_testkit::scrub_noob_env(&mut cmd);
    let out = cmd
        .env("NOOB_CONFIG_DIR", config.path())
        .env("NOOB_SANDBOX", "container")
        .env(
            "NOOB_WEBSEARCH",
            if with_websearch {
                websearch.as_os_str()
            } else {
                std::ffi::OsStr::new("off")
            },
        )
        .current_dir(work.path())
        .args(["debug", "prompt", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("debug prompt --json emits one JSON object")
}

fn tokens(text: &str) -> usize {
    let bpe = tiktoken_rs::o200k_base().unwrap();
    bpe.encode_with_special_tokens(text).len()
}

#[test]
fn no_output_cap_budget_and_phrasing() {
    let artifact = debug_prompt(false, false, false);
    let system = artifact["system"].as_str().unwrap();
    let head = artifact["head"].as_str().unwrap();
    let tools = artifact["tools"].to_string();

    // With no user prompt files, skills, or MCP, the system prompt IS the
    // head: the embedded AGENTS and TOOLS defaults plus the env block.
    assert_eq!(system, head);
    // 9 core (7 file/shell + context + todo) + subagent.
    assert_eq!(artifact["tools"].as_array().unwrap().len(), 10);

    let head_tokens = tokens(head);
    let tools_tokens = tokens(&tools);
    assert!(
        head_tokens <= HEAD_CEILING,
        "head is {head_tokens} tokens (ceiling {HEAD_CEILING})"
    );
    assert!(
        tools_tokens <= TOOLS_CEILING,
        "tools array is {tools_tokens} tokens (ceiling {TOOLS_CEILING})"
    );
    assert!(
        head_tokens + tools_tokens <= TOTAL_CEILING,
        "fixed overhead is {} tokens (ceiling {TOTAL_CEILING})",
        head_tokens + tools_tokens
    );

    // Forbidden cap phrasing: output is shaped by content instructions,
    // never by a length ceiling.
    let lower = system.to_lowercase();
    for banned in ["keep it brief", "keep it short", "be concise", "at most"] {
        assert!(
            !lower.contains(banned),
            "banned phrase {banned:?} in the prompt"
        );
    }
    let word_cap =
        regex::Regex::new(r"in \d+ (words|sentences|lines)|max \d+ (words|sentences)").unwrap();
    assert!(
        !word_cap.is_match(&lower),
        "cap-style phrasing in the prompt"
    );

    // And no max_tokens-family key anywhere near the wire.
    let tools_lower = tools.to_lowercase();
    assert!(!tools_lower.contains("max_tokens"));
    assert!(!tools_lower.contains("max_output_tokens"));
}

/// The ceilings hold for the full registered set: with a skill discovered,
/// MCP configured, and websearch installed, the tools array grows to 14
/// (9 core, subagent, skill, websearch, and the MCP pair). The system prompt
/// gains the resolver section and MCP line; the head itself stays byte-exact.
#[test]
fn budget_holds_with_everything_registered() {
    let artifact = debug_prompt(true, true, true);
    let system = artifact["system"].as_str().unwrap();
    let head = artifact["head"].as_str().unwrap();

    let tools = artifact["tools"].as_array().unwrap();
    let names = tools
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(tools.len(), 14, "{names:?}");
    for name in ["skill", "websearch", "mcp_connect", "mcp_call", "subagent"] {
        assert!(
            tools.iter().any(|t| t["function"]["name"] == name),
            "{name} must be registered"
        );
    }
    assert!(system.starts_with(head), "the head never mutates");
    assert!(system.contains("# Skills (resolver)"));
    assert!(system.contains("- budget-probe: a probe skill for the budget test"));
    assert!(system.contains("MCP servers (use mcp_connect): websearch"));

    let head_tokens = tokens(head);
    let system_tokens = tokens(system);
    let tools_tokens = tokens(&artifact["tools"].to_string());
    assert!(
        head_tokens <= HEAD_CEILING,
        "head is {head_tokens} tokens (ceiling {HEAD_CEILING})"
    );
    assert!(
        tools_tokens <= TOOLS_CEILING,
        "full tools array is {tools_tokens} tokens (ceiling {TOOLS_CEILING})"
    );
    assert!(
        system_tokens + tools_tokens <= TOTAL_CEILING,
        "fixed overhead is {} tokens: system {system_tokens} + tools {tools_tokens} \
         (ceiling {TOTAL_CEILING})",
        system_tokens + tools_tokens
    );
    assert!(
        system_tokens + tools_tokens <= OWNER_HARD_LIMIT,
        "the fixed prefix is {} tokens, over the owner's 2,000 hard limit",
        system_tokens + tools_tokens
    );
}

/// Descriptions stay terse, but a tool whose behavior the model cannot infer
/// from its name and parameters is allowed to explain that behavior here.
/// This is the right home for it: Anthropic's 2026 guidance is to put tool
/// guidance in the tool's own description rather than the system prompt, and
/// the shipped Claude Code binary does exactly that for its own read
/// short-circuit. The ceiling is per description, and TOOLS_CEILING still
/// bounds the total.
#[test]
fn tool_descriptions_stay_terse() {
    let artifact = debug_prompt(true, true, true);
    let tools = artifact["tools"].as_array().unwrap();
    let names = tools
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(tools.len(), 14, "{names:?}");
    for t in tools {
        let f = &t["function"];
        let desc = f["description"].as_str().unwrap();
        let words = desc.split_whitespace().count();
        assert!(
            words <= 45,
            "{} description has {words} words: {desc}",
            f["name"]
        );
    }
}
