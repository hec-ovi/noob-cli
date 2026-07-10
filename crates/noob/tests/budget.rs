//! Token-budget enforcement on the SHIPPED artifact: `noob debug prompt
//! --json` prints the exact system prompt and wire tools array the binary
//! sends; tiktoken o200k tokenizes them against the locked ceilings.
//! The live suite closes the loop against the real qwen tokenizer via
//! llama-server /tokenize (P7).

use std::process::Command;

use serde_json::Value;

// The locked budget, in one place so raising it is a visible diff
// (ARCHITECTURE.md, System prompt).
const HEAD_CEILING: usize = 560; // base.md + environment block
const TOOLS_CEILING: usize = 940; // serialized wire tools array
const TOTAL_CEILING: usize = 1500; // total fixed first-request overhead

/// `with_skill` plants one skill in the workspace and `with_mcp` one
/// configured server, so the artifact carries the FULL registered set (7
/// core + skill + mcp_connect + mcp_call) plus the resolver section and the
/// MCP line: the ceilings must hold with everything registered, not just
/// the bare core.
fn debug_prompt(with_skill: bool, with_mcp: bool) -> Value {
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
    let out = Command::new(env!("CARGO_BIN_EXE_noob"))
        .env("NOOB_CONFIG_DIR", config.path())
        .env("NOOB_SANDBOX", "container")
        .env_remove("NOOB_BASE_URL")
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
    let artifact = debug_prompt(false, false);
    let system = artifact["system"].as_str().unwrap();
    let head = artifact["head"].as_str().unwrap();
    let tools = artifact["tools"].to_string();

    // With no AGENTS.md, skills, or MCP, the system prompt IS the head.
    assert_eq!(system, head);
    assert_eq!(artifact["tools"].as_array().unwrap().len(), 7);

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
        assert!(!lower.contains(banned), "banned phrase {banned:?} in the prompt");
    }
    let word_cap = regex::Regex::new(r"in \d+ (words|sentences|lines)|max \d+ (words|sentences)")
        .unwrap();
    assert!(!word_cap.is_match(&lower), "cap-style phrasing in the prompt");

    // And no max_tokens-family key anywhere near the wire.
    let tools_lower = tools.to_lowercase();
    assert!(!tools_lower.contains("max_tokens"));
    assert!(!tools_lower.contains("max_output_tokens"));
}

/// The ceilings hold for the full registered set: with a skill discovered
/// and MCP configured the tools array grows to 10 (skill + the MCP pair),
/// the system prompt gains the resolver section and the MCP line; the head
/// itself must stay byte-identical.
#[test]
fn budget_holds_with_everything_registered() {
    let artifact = debug_prompt(true, true);
    let system = artifact["system"].as_str().unwrap();
    let head = artifact["head"].as_str().unwrap();

    let tools = artifact["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 10);
    for name in ["skill", "mcp_connect", "mcp_call"] {
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
    let tools_tokens = tokens(&artifact["tools"].to_string());
    assert!(
        head_tokens <= HEAD_CEILING,
        "head is {head_tokens} tokens (ceiling {HEAD_CEILING})"
    );
    assert!(
        tools_tokens <= TOOLS_CEILING,
        "full tools array is {tools_tokens} tokens (ceiling {TOOLS_CEILING})"
    );
    assert!(head_tokens + tools_tokens <= TOTAL_CEILING);
}

#[test]
fn tool_descriptions_stay_terse() {
    let artifact = debug_prompt(true, true);
    let tools = artifact["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 10);
    for t in tools {
        let f = &t["function"];
        let desc = f["description"].as_str().unwrap();
        let words = desc.split_whitespace().count();
        assert!(
            words <= 20,
            "{} description has {words} words: {desc}",
            f["name"]
        );
    }
}
