//! Model-callable context accounting. The estimate is the same conservative
//! value `/status` and automatic compaction use; it is informational and has
//! no authority to change the configured window.

use serde_json::{Value, json};

use noob_provider::types::ToolSpec;

use super::{ToolCtx, ToolOutcome};

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "context".to_string(),
        description:
            "Report estimated context use, total window, and the automatic compaction threshold."
                .to_string(),
        parameters: json!({"type": "object", "properties": {}}),
    }
}

pub fn run(ctx: &ToolCtx, _args: &Value) -> ToolOutcome {
    let (used, total) = ctx.context();
    ToolOutcome::ok(
        report(used, total),
        format!("context: {}/{}", token_label(used), token_label(total)),
    )
}

/// The one-line usage report, shared by the model-callable tool and the
/// human-facing `/context` command so the two can never drift.
pub fn report(used: u64, total: u64) -> String {
    let pct = used.saturating_mul(100) / total.max(1);
    let threshold = total.saturating_mul(3) / 4;
    format!(
        "context: ~{} / {} tokens ({pct}%); automatic compaction starts near {} (75%)",
        token_label(used),
        token_label(total),
        token_label(threshold),
    )
}

pub(crate) fn token_label(tokens: u64) -> String {
    if tokens < 1_000 {
        tokens.to_string()
    } else {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_ctx;

    #[test]
    fn reports_the_shared_estimate_total_and_threshold() {
        let (_tmp, ctx) = test_ctx();
        ctx.set_context(83_153, 131_072);
        let out = run(&ctx, &json!({}));
        assert!(!out.is_error);
        assert!(
            out.content.contains("83.2k / 131.1k tokens (63%)"),
            "{}",
            out.content
        );
        assert!(out.content.contains("98.3k (75%)"), "{}", out.content);
        assert_eq!(out.summary, "context: 83.2k/131.1k");
    }

    #[test]
    fn token_labels_keep_small_values_exact() {
        assert_eq!(token_label(999), "999");
        assert_eq!(token_label(4_096), "4.1k");
    }
}
