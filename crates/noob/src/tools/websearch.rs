//! websearch: the web, as one tool instead of one server.
//!
//! Registered when the `websearch` CLI is on PATH. It is the whole web
//! capability of a research child (`tools: "web"`), which is why it exists as
//! a first-class tool rather than a line in the skill telling the model to run
//! bash: a child that can run bash can run anything, and the evidence gate
//! needs something countable.
//!
//! Every call is a fixed argv built from validated fields, executed directly
//! with no shell. The model never supplies a flag, a command string, or a
//! value beginning with `-`, so there is nothing to smuggle into the CLI's
//! own parser. Output comes back wrapped as untrusted, because search results
//! and page text are written by strangers.

use std::process::Command;

use serde_json::{Value, json};

use noob_provider::types::ToolSpec;

use super::truncate::mcp_cap;
use super::untrusted;
use super::{ToolCtx, ToolOutcome, need_str, opt_u64};

/// The command noob shells out to. Installed in the runtime image as a uv
/// tool; on a host it comes from `uv tool install websearch-skill`.
pub const PROGRAM: &str = "websearch";

/// Override for the resolved binary: a path to run instead of searching PATH,
/// or an off word to unregister the tool entirely. Tests point it at a stub;
/// a user who prefers their own web tooling can turn this one off with it.
pub const OVERRIDE_VAR: &str = "NOOB_WEBSEARCH";

/// Per-action defaults. `init` clones and builds SearXNG on a cold state
/// directory, which is a minute of real work, and `doctor` probes every
/// engine in series; a 120s default would report those as timeouts.
const INIT_TIMEOUT_S: u64 = 420;
const DOCTOR_TIMEOUT_S: u64 = 240;
const DEFAULT_TIMEOUT_S: u64 = 90;
const MAX_TIMEOUT_S: u64 = 600;

/// The binary to run, or None when the tool is unavailable (or turned off).
/// Resolved once: the answer decides tool registration at bootstrap and cannot
/// change mid-session.
pub fn program() -> Option<&'static std::path::Path> {
    static FOUND: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| match std::env::var(OVERRIDE_VAR) {
            Ok(value)
                if matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "" | "0" | "off" | "no" | "false"
                ) =>
            {
                None
            }
            Ok(value) => {
                let path = std::path::PathBuf::from(value.trim());
                std::fs::metadata(&path)
                    .is_ok_and(|m| m.is_file())
                    .then_some(path)
            }
            Err(_) => which(PROGRAM),
        })
        .as_deref()
}

/// Is the CLI installed?
pub fn available() -> bool {
    program().is_some()
}

fn which(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| std::fs::metadata(candidate).is_ok_and(|m| m.is_file()))
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "websearch".to_string(),
        description:
            "Search the web, fetch a page as clean Markdown, or find papers and repositories. \
             Run action=\"init\" once before searching to start SearXNG and check capabilities. \
             Use search to find pages, fetch to read one, and open with a prior handle to \
             continue a document."
                .to_string(),
        parameters: json!({"type": "object", "properties": {
            "action": {
                "type": "string",
                "enum": ["init", "search", "fetch", "open", "arxiv", "github", "doctor"],
                "description": "init: bring the tool online and report what works. \
                                search: find pages. fetch: read one URL. open: another page \
                                of a fetched document. arxiv/github: papers, repositories. \
                                doctor: why searches are coming back empty."
            },
            "query": {"type": "string", "description": "search/arxiv/github: what to look for"},
            "url": {"type": "string", "description": "fetch: an absolute http(s) URL"},
            "handle": {"type": "string", "description": "open: a handle from an earlier result"},
            "page": {"type": "integer", "description": "fetch/open: 1-based page, default 1"},
            "max_results": {"type": "integer", "description": "search/arxiv: how many, default 8"},
            "site": {"type": "string", "description": "search: restrict to one host"},
            "freshness": {
                "type": "string",
                "enum": ["any", "day", "week", "month", "year"],
                "description": "search: recency filter, best effort"
            },
            "language": {"type": "string", "description": "github: restrict to a language"},
            "timeout_s": {"type": "integer", "description": "seconds, max 600"}
        }, "required": ["action"]}),
    }
}

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    match run_inner(ctx, args) {
        Ok(out) => out,
        Err(message) if message.starts_with("websearch canceled") => {
            ToolOutcome::canceled_with(message)
        }
        Err(message) => ToolOutcome::err(message),
    }
}

/// A field that becomes one argv entry. Rejecting a leading `-` is what keeps
/// a value from being read as a flag by the CLI's own parser; the rest is
/// ordinary emptiness and control-character hygiene.
fn value(args: &Value, key: &str) -> Result<String, String> {
    let raw = need_str(args, key)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{key} is empty"));
    }
    if trimmed.starts_with('-') {
        return Err(format!(
            "{key} must not start with '-': that would be read as a flag, and this tool \
             takes no flags"
        ));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(format!("{key} contains a control character"));
    }
    Ok(trimmed.to_string())
}

fn optional(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => value(args, key).map(Some),
    }
}

fn count(args: &Value, key: &str, max: u64) -> Result<Option<u64>, String> {
    match opt_u64(args, key)? {
        None => Ok(None),
        Some(n) if n > max => Err(format!("{key} is at most {max}")),
        Some(n) => Ok(Some(n)),
    }
}

fn run_inner(ctx: &ToolCtx, args: &Value) -> Result<ToolOutcome, String> {
    let action = value(args, "action")?;
    let mut argv: Vec<String> = Vec::new();
    // Whether a successful call counts as consulting a source. init and doctor
    // report on the installation; they are not evidence of research.
    let mut evidence = true;
    match action.as_str() {
        "init" => {
            argv.push("init".into());
            evidence = false;
        }
        "doctor" => {
            argv.push("doctor".into());
            evidence = false;
        }
        "search" => {
            argv.push("web-search".into());
            argv.push(value(args, "query")?);
            if let Some(n) = count(args, "max_results", 50)? {
                argv.push("--max-results".into());
                argv.push(n.to_string());
            }
            if let Some(site) = optional(args, "site")? {
                argv.push("--site".into());
                argv.push(site);
            }
            if let Some(freshness) = optional(args, "freshness")? {
                if !["any", "day", "week", "month", "year"].contains(&freshness.as_str()) {
                    return Err(format!(
                        "freshness {freshness:?} is not one of any, day, week, month, year"
                    ));
                }
                argv.push("--freshness".into());
                argv.push(freshness);
            }
        }
        "fetch" => {
            let url = value(args, "url")?;
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err("url must be an absolute http(s) URL".to_string());
            }
            argv.push("web-fetch".into());
            argv.push(url);
            if let Some(page) = count(args, "page", 10_000)? {
                argv.push("--page".into());
                argv.push(page.max(1).to_string());
            }
        }
        "open" => {
            argv.push("web-open".into());
            argv.push(value(args, "handle")?);
            if let Some(page) = count(args, "page", 10_000)? {
                argv.push("--page".into());
                argv.push(page.max(1).to_string());
            }
        }
        "arxiv" => {
            argv.push("arxiv".into());
            argv.push(value(args, "query")?);
            if let Some(n) = count(args, "max_results", 50)? {
                argv.push("--max-results".into());
                argv.push(n.to_string());
            }
        }
        "github" => {
            argv.push("github".into());
            argv.push(value(args, "query")?);
            if let Some(language) = optional(args, "language")? {
                argv.push("--language".into());
                argv.push(language);
            }
        }
        other => {
            return Err(format!(
                "unknown action {other:?}; use init, search, fetch, open, arxiv, github, or doctor"
            ));
        }
    }

    let default_timeout = match action.as_str() {
        "init" => INIT_TIMEOUT_S,
        "doctor" => DOCTOR_TIMEOUT_S,
        _ => DEFAULT_TIMEOUT_S,
    };
    let timeout_s = opt_u64(args, "timeout_s")?
        .unwrap_or(default_timeout)
        .clamp(1, MAX_TIMEOUT_S);

    let Some(program) = program() else {
        return Err(format!(
            "the {PROGRAM:?} CLI is not available in this session; install it with \
             `uv tool install websearch-skill`"
        ));
    };
    let mut command = Command::new(program);
    command.args(&argv).current_dir(&ctx.workspace);
    let run = match super::exec::run(
        command,
        PROGRAM,
        timeout_s,
        ctx.caps.mcp_head,
        ctx.caps.mcp_tail,
    ) {
        Ok(run) => run,
        Err(super::exec::RunError::Spawn(message)) => {
            return Err(format!(
                "{message}. Install it with `uv tool install websearch-skill`, or use bash if \
                 this session has it."
            ));
        }
        Err(super::exec::RunError::Canceled { body, elapsed }) => {
            return Err(format!(
                "websearch canceled by user after {:.1}s; partial output:\n{body}",
                elapsed.as_secs_f32()
            ));
        }
        Err(super::exec::RunError::TimedOut { body, timeout_s }) => {
            return Err(format!(
                "websearch {action} timed out after {timeout_s}s and was killed; raise \
                 timeout_s (max {MAX_TIMEOUT_S}); partial output:\n{body}"
            ));
        }
    };

    let summary = format!(
        "websearch {action} ({:.1}s, exit {})",
        run.elapsed.as_secs_f32(),
        run.code
    );
    if run.code != 0 {
        // The CLI reports a failure as a clean one-line error on stderr, which
        // is already in the body. Exit 1 from `doctor` or `init` means "found
        // problems", not "the tool broke", so the body is the answer either way.
        let mut out = ToolOutcome::err(untrusted::wrap(
            "the web (websearch)",
            &mcp_cap(&run.body, &ctx.caps),
        ));
        out.summary = summary;
        return Ok(out);
    }
    if evidence {
        ctx.record_evidence_call();
    }
    let body = if run.body.trim().is_empty() {
        "(no output)".to_string()
    } else {
        run.body
    };
    Ok(ToolOutcome::ok(
        untrusted::wrap("the web (websearch)", &mcp_cap(&body, &ctx.caps)),
        summary,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_ctx;

    #[test]
    fn a_value_that_looks_like_a_flag_is_refused() {
        let (_tmp, ctx) = test_ctx();
        let out = run(&ctx, &json!({"action": "search", "query": "--json"}));
        assert!(out.is_error);
        assert!(
            out.content.contains("must not start with '-'"),
            "{}",
            out.content
        );
    }

    #[test]
    fn an_unknown_action_lists_the_real_ones() {
        let (_tmp, ctx) = test_ctx();
        let out = run(&ctx, &json!({"action": "rm-rf"}));
        assert!(out.is_error);
        assert!(out.content.contains("unknown action"), "{}", out.content);
        assert!(
            out.content.contains("init, search, fetch"),
            "{}",
            out.content
        );
    }

    #[test]
    fn fetch_requires_an_absolute_http_url() {
        let (_tmp, ctx) = test_ctx();
        for url in ["file:///etc/passwd", "example.com/page"] {
            let out = run(&ctx, &json!({"action": "fetch", "url": url}));
            assert!(out.is_error, "{url} was accepted");
            assert!(
                out.content.contains("absolute http(s) URL"),
                "{}",
                out.content
            );
        }
    }

    #[test]
    fn a_missing_required_field_names_it() {
        let (_tmp, ctx) = test_ctx();
        let out = run(&ctx, &json!({"action": "search"}));
        assert!(out.is_error);
        assert!(out.content.contains("query"), "{}", out.content);
    }

    #[test]
    fn a_control_character_is_refused() {
        let (_tmp, ctx) = test_ctx();
        let out = run(
            &ctx,
            &json!({"action": "search", "query": "rust\nownership"}),
        );
        assert!(out.is_error);
        assert!(out.content.contains("control character"), "{}", out.content);
    }

    #[test]
    fn an_out_of_range_count_is_refused() {
        let (_tmp, ctx) = test_ctx();
        let out = run(
            &ctx,
            &json!({"action": "search", "query": "rust", "max_results": 5000}),
        );
        assert!(out.is_error);
        assert!(out.content.contains("at most 50"), "{}", out.content);
    }

    #[test]
    fn a_bad_freshness_is_refused_before_the_process_starts() {
        let (_tmp, ctx) = test_ctx();
        let out = run(
            &ctx,
            &json!({"action": "search", "query": "rust", "freshness": "yesterday"}),
        );
        assert!(out.is_error);
        assert!(
            out.content.contains("not one of any, day"),
            "{}",
            out.content
        );
    }

    #[test]
    fn the_spec_names_init_first() {
        let spec = spec();
        assert_eq!(spec.name, "websearch");
        assert!(
            spec.description.contains("action=\"init\""),
            "{}",
            spec.description
        );
        let actions = spec.parameters["properties"]["action"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(actions[0], "init");
    }

    #[test]
    fn a_validation_error_never_records_evidence() {
        let (_tmp, ctx) = test_ctx();
        run(&ctx, &json!({"action": "search", "query": "--flag"}));
        assert_eq!(ctx.evidence_call_count(), 0);
    }
}
