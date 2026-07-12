//! todo: an agentic, visible checklist the model maintains as it works (like
//! Claude Code's TodoWrite). The model calls it itself; there is no approval
//! ceremony. Overwrite-whole-list semantics: each call carries the full
//! current plan, which replaces the stored one. The rendered checklist is the
//! tool result, so every surface and the model transcript see the plan; the
//! interactive themed REPL styles the glyphs on top of the exact same text.
//!
//! Not a plan-MODE tool: it mutates shared state, so it is a sequential
//! barrier (never in READ_ONLY_SET) and is not wired into `/plan`.

use serde_json::{Value, json};

use noob_provider::types::ToolSpec;

use super::{TodoItem, TodoStatus, ToolCtx, ToolOutcome};

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "todo".to_string(),
        description: "Maintain a visible checklist for a multi-step task; send the whole updated \
                      list each call."
            .to_string(),
        parameters: json!({"type": "object", "properties": {
            "todos": {"type": "array", "items": {"type": "object", "properties": {
                "content": {"type": "string"},
                "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]}
            }, "required": ["content", "status"]}}
        }, "required": ["todos"]}),
    }
}

pub fn run(ctx: &ToolCtx, args: &Value) -> ToolOutcome {
    let raw = match args.get("todos") {
        None | Some(Value::Null) => {
            return ToolOutcome::err(
                "missing required parameter \"todos\" (an array of {content, status}); \
                 resend the call",
            );
        }
        Some(Value::Array(items)) => items,
        Some(other) => {
            return ToolOutcome::err(format!(
                "parameter \"todos\" must be an array of {{content, status}}, got {other}; \
                 resend the call"
            ));
        }
    };

    let mut parsed = Vec::with_capacity(raw.len());
    for (i, item) in raw.iter().enumerate() {
        let content = match item.get("content") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
            Some(Value::String(_)) => {
                return ToolOutcome::err(format!(
                    "todos[{i}].content is empty; every item needs a short non-empty description"
                ));
            }
            _ => {
                return ToolOutcome::err(format!(
                    "todos[{i}] is missing a string \"content\"; resend the call"
                ));
            }
        };
        let status = match item.get("status").and_then(Value::as_str) {
            Some(s) => match TodoStatus::parse(s) {
                Some(st) => st,
                None => {
                    return ToolOutcome::err(format!(
                        "todos[{i}].status is {s:?}; use \"pending\", \"in_progress\", \
                         or \"completed\""
                    ));
                }
            },
            None => {
                return ToolOutcome::err(format!(
                    "todos[{i}] is missing a string \"status\" \
                     (pending, in_progress, or completed); resend the call"
                ));
            }
        };
        parsed.push(TodoItem { content, status });
    }

    let total = parsed.len();
    let done = parsed
        .iter()
        .filter(|t| t.status == TodoStatus::Completed)
        .count();
    let content = render(&parsed);

    // Overwrite the whole list: the model always sends the full current plan.
    *ctx.todos.lock().unwrap() = parsed;

    ToolOutcome::ok(content, format!("plan: {done}/{total} done"))
}

/// The plain checklist text: a header line, then one `glyph content` line per
/// item. This IS the tool result verbatim (every surface and the model see
/// it); the themed REPL recolors the glyph lines on top of this exact text.
/// Item content is flattened to one line so each row stays a single glyph line.
pub fn render(items: &[TodoItem]) -> String {
    let total = items.len();
    let done = items
        .iter()
        .filter(|t| t.status == TodoStatus::Completed)
        .count();
    let mut out = format!("plan ({done}/{total} done):");
    for item in items {
        let flat = item.content.replace(['\n', '\r'], " ");
        out.push('\n');
        out.push_str(item.status.glyph());
        out.push(' ');
        out.push_str(flat.trim_end());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_ctx;

    #[test]
    fn overwrites_the_list_and_reports_progress() {
        let (_tmp, ctx) = test_ctx();
        let out = run(
            &ctx,
            &json!({"todos": [
                {"content": "research the codebase", "status": "completed"},
                {"content": "write the todo tool", "status": "in_progress"},
                {"content": "add tests", "status": "pending"}
            ]}),
        );
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.summary, "plan: 1/3 done");
        // The rendered result carries the header, every glyph, and every item.
        assert!(out.content.starts_with("plan (1/3 done):\n"));
        assert!(out.content.contains("[x] research the codebase"));
        assert!(out.content.contains("[~] write the todo tool"));
        assert!(out.content.contains("[ ] add tests"));
        assert_eq!(ctx.todos.lock().unwrap().len(), 3);

        // A second call overwrites the whole list (not appends) and the
        // progress re-renders with the advanced status.
        let out = run(
            &ctx,
            &json!({"todos": [
                {"content": "research the codebase", "status": "completed"},
                {"content": "write the todo tool", "status": "completed"},
                {"content": "add tests", "status": "in_progress"}
            ]}),
        );
        assert_eq!(out.summary, "plan: 2/3 done");
        assert!(out.content.contains("[x] write the todo tool"));
        assert!(out.content.contains("[~] add tests"));
        assert_eq!(ctx.todos.lock().unwrap().len(), 3, "list was overwritten, not grown");
    }

    #[test]
    fn bad_status_is_a_typed_error_that_teaches() {
        let (_tmp, ctx) = test_ctx();
        let out = run(
            &ctx,
            &json!({"todos": [{"content": "x", "status": "doing"}]}),
        );
        assert!(out.is_error);
        assert!(out.content.contains("todos[0].status is \"doing\""), "{}", out.content);
        assert!(out.content.contains("in_progress"));
        // A rejected call never touched the stored list.
        assert!(ctx.todos.lock().unwrap().is_empty());
    }

    #[test]
    fn missing_pieces_are_typed_errors() {
        let (_tmp, ctx) = test_ctx();
        assert!(run(&ctx, &json!({})).content.contains("missing required parameter \"todos\""));
        assert!(run(&ctx, &json!({"todos": "nope"})).content.contains("must be an array"));
        let out = run(&ctx, &json!({"todos": [{"status": "pending"}]}));
        assert!(out.content.contains("todos[0] is missing a string \"content\""), "{}", out.content);
        let out = run(&ctx, &json!({"todos": [{"content": "  "}]}));
        assert!(out.content.contains("todos[0].content is empty"), "{}", out.content);
        let out = run(&ctx, &json!({"todos": [{"content": "a"}]}));
        assert!(out.content.contains("todos[0] is missing a string \"status\""), "{}", out.content);
    }

    #[test]
    fn empty_list_clears_the_plan() {
        let (_tmp, ctx) = test_ctx();
        run(&ctx, &json!({"todos": [{"content": "a", "status": "pending"}]}));
        let out = run(&ctx, &json!({"todos": []}));
        assert!(!out.is_error);
        assert_eq!(out.summary, "plan: 0/0 done");
        assert!(ctx.todos.lock().unwrap().is_empty());
    }

    #[test]
    fn render_flattens_multiline_content_to_one_glyph_line() {
        let items = vec![TodoItem {
            content: "line one\nline two".to_string(),
            status: TodoStatus::Pending,
        }];
        let text = render(&items);
        assert_eq!(text.lines().count(), 2, "header + one item line: {text:?}");
        assert!(text.contains("[ ] line one line two"));
    }

    #[test]
    fn spec_stays_terse() {
        let s = spec();
        assert_eq!(s.name, "todo");
        assert!(s.description.split_whitespace().count() <= 20);
        assert!(s.parameters.get("type").is_some());
    }
}
