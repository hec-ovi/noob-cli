//! skill: level-2 disclosure. Returns the SKILL.md body (frontmatter
//! stripped) plus the skill's directory path as an ordinary tool result, so
//! the prompt head never mutates when a skill loads. Registered only when
//! discovery found at least one skill. Skill bodies are untrusted input.

use std::sync::atomic::Ordering;

use noob_provider::http::INTERRUPTED;
use serde_json::{Value, json};

use noob_provider::types::ToolSpec;

use super::truncate::{floor_char_boundary, skill_cap_marker};
use super::{ToolCtx, ToolOutcome, display_path, need_str};

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "skill".to_string(),
        description: "Load a skill by name; the available skills are listed in the system prompt."
            .to_string(),
        parameters: json!({"type": "object", "properties": {
            "name": {"type": "string"}
        }, "required": ["name"]}),
    }
}

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    run_with(ctx, args, || INTERRUPTED.load(Ordering::SeqCst))
}

fn run_with(ctx: &ToolCtx, args: &Value, interrupted: impl Fn() -> bool) -> ToolOutcome {
    if interrupted() {
        return ToolOutcome::canceled();
    }
    let name = match need_str(args, "name") {
        Ok(n) => n,
        Err(e) => return ToolOutcome::err(e),
    };
    let Some(skill) = ctx.skills.iter().find(|s| s.name == name) else {
        let available: Vec<&str> = ctx.skills.iter().map(|s| s.name.as_str()).collect();
        return ToolOutcome::err(format!(
            "unknown skill {name:?}; available skills: {}",
            available.join(", ")
        ));
    };
    let text = match std::fs::read_to_string(&skill.file) {
        Ok(t) => t,
        Err(e) => {
            return ToolOutcome::err(format!(
                "cannot read skill {name:?} at {}: {e}",
                skill.file.display()
            ));
        }
    };
    if interrupted() {
        return ToolOutcome::canceled();
    }
    let (body, frontmatter_lines) = crate::skills::body_of(&text);
    let body = body.as_ref();

    let mut shown = body;
    let mut marker = String::new();
    if body.len() > ctx.caps.skill_bytes {
        let cut = floor_char_boundary(body, ctx.caps.skill_bytes);
        shown = &body[..cut];
        // Continue on the file line holding the cut (re-reading a partial
        // line beats losing it); file lines are 1-based.
        let next_line = frontmatter_lines + shown.matches('\n').count() + 1;
        marker = format!(
            "\n{}",
            skill_cap_marker(&display_path(ctx, &skill.file), next_line)
        );
    }
    // The ~5k-token recommendation from the skills standard, echoed as a
    // UI-only warning so oversize bodies get noticed without failing.
    let token_estimate = body.len() / 4;
    let warning = (token_estimate > 5_000).then(|| {
        format!(
            "skill {name} is ~{token_estimate} tokens; the skills standard recommends \
             bodies under 5000 tokens"
        )
    });

    {
        let mut loaded = ctx.loaded_skills.lock().unwrap();
        if !loaded.iter().any(|n| n == name) {
            loaded.push(name.to_string());
        }
    }

    let dir = display_path(ctx, &skill.dir);
    let lines = shown.lines().count();
    // Skills are portable documents and may name another harness's tool
    // vocabulary. Give the model a mechanical mapping before the untrusted
    // body so compatibility does not depend on improvisation. Loaded skill
    // names are also withheld from descendants by the subagent protocol.
    // ctx.task is Some exactly when the subagent tool is registered here.
    // Advertising delegation where the schema is absent (a leaf child, the
    // depth ceiling) steered small models into refused calls.
    let runtime = if ctx.task.is_some() {
        "[noob runtime mapping: use subagent(prompt, tools, max_turns) for delegation; \
         dock subagents are already detached, so ignore model, description, and \
         run_in_background fields from other harnesses. Translate foreign web-tool \
         names to registered noob tools. Use tools:\"web\" for a nonmutating research \
         child with web MCP access. This loaded skill is not exposed again inside \
         descendants.]"
    } else {
        "[noob runtime mapping: delegation (subagent, agents, tasks) is unavailable in \
         this context; do the delegated work yourself with your registered tools. \
         Translate foreign web-tool names to registered noob tools.]"
    };
    // Repeat the operational part after the untrusted body. Large skills can
    // contain many later harness-specific directives, and small local models
    // follow the nearest instruction more reliably than an early disclaimer.
    let reminder = "[noob runtime reminder: child briefs may name only tools registered in this \
                    session. When MCP server websearch is configured, say to call mcp_connect \
                    {\"server\":\"websearch\"} once and then mcp_call, and pass tools:\"web\"; \
                    this prevents Bash/write/edit, so the child must return its complete synthesis \
                    and never create files. Do not tell it to load a web-search skill or use \
                    WebSearch/WebFetch. Otherwise say to use Bash websearch/curl. A user-specified \
                    agent count is a hard cap; never spawn a replacement after failure unless the \
                    user explicitly requested retries.]";
    ToolOutcome {
        content: format!("skill: {name}\ndir: {dir}\n\n{runtime}\n\n{shown}{marker}\n\n{reminder}"),
        is_error: false,
        summary: format!("skill {name} ({lines} lines)"),
        warning,
        canceled: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::Skill;
    use crate::tools::test_ctx;

    fn install_skill(ctx: &mut ToolCtx, name: &str, body: &str) {
        let dir = ctx.workspace.join(".noob/skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("SKILL.md");
        std::fs::write(
            &file,
            format!("---\nname: {name}\ndescription: test skill\n---\n{body}"),
        )
        .unwrap();
        ctx.skills.push(Skill {
            name: name.to_string(),
            description: "test skill".to_string(),
            dir,
            file,
        });
    }

    #[test]
    fn returns_body_without_frontmatter_plus_dir_and_tracks_loading() {
        let (_tmp, mut ctx) = test_ctx();
        install_skill(
            &mut ctx,
            "pdf-tools",
            "# PDF tools\n\nUse pdftotext first.\n",
        );
        // The subagent tool is registered on this surface, so the mapping
        // advertises delegation.
        ctx.task = Some(crate::subagent::TaskCfg {
            depth: 0,
            concurrency: crate::subagent::DEFAULT_CONCURRENCY,
            max_turns: crate::subagent::DEFAULT_MAX_TURNS,
            wall_clock: std::time::Duration::from_secs(crate::subagent::DEFAULT_WALL_CLOCK_S),
            verbose: false,
            overrides: Default::default(),
            yolo: false,
            ancestor_skills: Vec::new(),
            background: None,
        });
        let out = run(&ctx, &json!({"name": "pdf-tools"}));
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content
                .starts_with("skill: pdf-tools\ndir: .noob/skills/pdf-tools\n\n")
        );
        assert!(out.content.contains("Use pdftotext first."));
        assert!(out.content.contains("[noob runtime mapping:"));
        assert!(out.content.contains("[noob runtime reminder:"));
        assert!(out.content.ends_with("]"));
        assert!(out.content.contains("mcp_connect"));
        assert!(out.content.contains("then mcp_call"));
        assert!(out.content.contains("tools:\"web\""));
        assert!(out.content.contains("never create files"));
        assert!(out.content.contains("run_in_background"));
        assert!(
            !out.content.contains("description: test skill"),
            "frontmatter must be stripped"
        );
        // Without a registered subagent tool (a leaf child, the depth
        // ceiling) the mapping must not advertise delegation the schema
        // cannot honor.
        ctx.task = None;
        let leaf = run(&ctx, &json!({"name": "pdf-tools"}));
        assert!(!leaf.is_error, "{}", leaf.content);
        assert!(
            leaf.content
                .contains("delegation (subagent, agents, tasks) is unavailable"),
            "{}",
            leaf.content
        );
        assert!(
            !leaf.content.contains("use subagent(prompt"),
            "a leaf must not be steered into refused delegation calls: {}",
            leaf.content
        );
        assert!(out.warning.is_none());
        assert_eq!(
            *ctx.loaded_skills.lock().unwrap(),
            vec!["pdf-tools".to_string()]
        );
        // Loading again does not duplicate the tracking entry.
        run(&ctx, &json!({"name": "pdf-tools"}));
        assert_eq!(ctx.loaded_skills.lock().unwrap().len(), 1);
    }

    #[test]
    fn unknown_skill_lists_the_available_ones() {
        let (_tmp, mut ctx) = test_ctx();
        install_skill(&mut ctx, "alpha", "a\n");
        install_skill(&mut ctx, "beta", "b\n");
        let out = run(&ctx, &json!({"name": "gamma"}));
        assert!(out.is_error);
        assert!(out.content.contains("unknown skill \"gamma\""));
        assert!(out.content.contains("alpha, beta"));
        assert!(ctx.loaded_skills.lock().unwrap().is_empty());
    }

    #[test]
    fn load_cancellation_is_structural() {
        let (_tmp, mut ctx) = test_ctx();
        install_skill(&mut ctx, "alpha", "body\n");
        let out = run_with(&ctx, &json!({"name": "alpha"}), || true);
        assert!(out.canceled);
        assert!(out.is_error);
        assert!(ctx.loaded_skills.lock().unwrap().is_empty());
    }

    #[test]
    fn uncapped_ctx_loads_the_whole_oversize_body() {
        let (_tmp, mut ctx) = test_ctx();
        ctx.caps = crate::tools::truncate::Caps::uncapped();
        // The same 30 KiB fixture that gets cut at 24 KiB under the defaults.
        let body: String = (0..3000).map(|i| format!("body line {i}\n")).collect();
        install_skill(&mut ctx, "big", &body);
        let out = run(&ctx, &json!({"name": "big"}));
        assert!(!out.is_error);
        assert!(!out.content.contains("[skill body capped"));
        assert!(out.content.contains("body line 2999\n"));
        // The oversize UI warning is about the skills standard, not the cap,
        // so it still fires.
        assert!(out.warning.is_some());
    }

    #[test]
    fn oversize_body_is_capped_with_a_read_pointer_and_a_warning() {
        let (_tmp, mut ctx) = test_ctx();
        // 30 KiB body: past the 24 KiB cap and the ~5k-token recommendation.
        let body: String = (0..3000).map(|i| format!("body line {i}\n")).collect();
        install_skill(&mut ctx, "big", &body);
        let out = run(&ctx, &json!({"name": "big"}));
        assert!(!out.is_error);
        assert!(out.content.len() < 25 * 1024 + 200);
        // The cap must deliver the leading ~24 KiB, not an empty stub.
        assert!(
            out.content.len() > 24 * 1024 - 200,
            "capped body suspiciously small"
        );
        assert!(
            out.content.contains("body line 0\n"),
            "the body head must survive the cap"
        );
        assert!(
            out.content
                .contains("[skill body capped at 24 KiB; read the rest with read ")
        );
        assert!(out.content.contains(".noob/skills/big/SKILL.md offset="));
        let warning = out.warning.expect("oversize warning");
        assert!(
            warning.contains("recommends bodies under 5000 tokens"),
            "{warning}"
        );
        // The estimate is the real chars/4 figure, not a floored "5k" that
        // reads as equal to the recommendation.
        let est: u64 = warning
            .split_once("is ~")
            .and_then(|(_, rest)| rest.split(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("no numeric estimate in: {warning}"));
        assert!(est > 5_000, "estimate {est} must exceed the recommendation");
        // The pointer's offset continues where the cap landed: frontmatter
        // (4 lines) + full body lines shown + 1.
        let offset: usize = out
            .content
            .rsplit_once("offset=")
            .unwrap()
            .1
            .split_once(']')
            .unwrap()
            .0
            .parse()
            .unwrap();
        let shown_newlines = out
            .content
            .split_once("\n\n")
            .unwrap()
            .1
            .split_once("\n\n")
            .unwrap()
            .1
            .rsplit_once("\n[skill body capped")
            .unwrap()
            .0
            .matches('\n')
            .count();
        assert_eq!(offset, 4 + shown_newlines + 1);
    }

    #[test]
    fn missing_name_and_missing_file_are_typed_errors() {
        let (_tmp, mut ctx) = test_ctx();
        install_skill(&mut ctx, "gone", "body\n");
        let out = run(&ctx, &json!({}));
        assert!(out.is_error);
        assert!(out.content.contains("missing required parameter"));
        std::fs::remove_file(&ctx.skills[0].file).unwrap();
        let out = run(&ctx, &json!({"name": "gone"}));
        assert!(out.is_error);
        assert!(out.content.contains("cannot read skill"));
    }

    #[test]
    fn spec_stays_terse() {
        let s = spec();
        assert!(s.description.split_whitespace().count() <= 20);
        assert_eq!(s.name, "skill");
    }
}
