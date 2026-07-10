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

fn debug_prompt() -> Value {
    let config = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(config.path().join(".env"), "NOOB_MODEL=qwen3.6-35b-a3b\n").unwrap();
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
    let artifact = debug_prompt();
    let system = artifact["system"].as_str().unwrap();
    let head = artifact["head"].as_str().unwrap();
    let tools = artifact["tools"].to_string();

    // With no AGENTS.md, skills, or MCP, the system prompt IS the head.
    assert_eq!(system, head);

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

#[test]
fn tool_descriptions_stay_terse() {
    let artifact = debug_prompt();
    let tools = artifact["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 7);
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
