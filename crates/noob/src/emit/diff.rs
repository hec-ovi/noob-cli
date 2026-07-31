//! The changed middle between two versions of a file, framed for `file.edit`.

/// The lines both sides agree on: a common prefix and a common suffix, which
/// leaves the changed middle between them.
fn unchanged_ends(before: &[&str], after: &[&str]) -> (usize, usize) {
    let head = before
        .iter()
        .zip(after)
        .take_while(|(a, b)| a == b)
        .count();
    let tail = before
        .iter()
        .rev()
        .zip(after.iter().rev())
        .take_while(|(a, b)| a == b)
        .take(before.len().min(after.len()) - head)
        .count();
    (head, tail)
}

/// The line span a replacement occupies, and what it replaced.
///
/// Both sides of an edit are already in scope wherever a tool writes a file, so
/// a consumer never has to re-read anything to draw a diff. Lines are 1-based
/// and inclusive, matching `read`'s own numbering.
pub fn edit_span(before: &str, after: &str) -> noob_proto::Span {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let (head, tail) = unchanged_ends(&before_lines, &after_lines);
    let start = head + 1;
    let end = after_lines.len().saturating_sub(tail).max(head);
    noob_proto::Span {
        start: start as u32,
        end: end.max(start.saturating_sub(1)) as u32,
        kind: None,
        name: None,
    }
}

/// One `file.edit` frame for a whole-file replacement.
///
/// The span is in the written file's coordinates, and the two texts are the
/// same region on either side, so a consumer draws the diff from the frame
/// alone. Clipping to the changed middle is what makes this affordable: a
/// one-line fix in a 3,000-line file sends two lines, not six thousand.
pub fn file_edit(
    path: String,
    before: &str,
    after: &str,
    call_id: Option<String>,
) -> noob_proto::Event {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let (head, tail) = unchanged_ends(&before_lines, &after_lines);
    let before_end = before_lines.len().saturating_sub(tail);
    let after_end = after_lines.len().saturating_sub(tail);
    noob_proto::Event::FileEdit {
        path,
        span: edit_span(before, after),
        before: before_lines[head.min(before_end)..before_end].join("\n"),
        after: after_lines[head.min(after_end)..after_end].join("\n"),
        call_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_span_covers_only_the_changed_lines() {
        // One line changed in the middle.
        let span = edit_span("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!((span.start, span.end), (2, 2));
        // A replacement that grows.
        let span = edit_span("a\nb\nc\n", "a\nX\nY\nc\n");
        assert_eq!((span.start, span.end), (2, 3));
        // A change at the very start, and at the very end.
        assert_eq!(edit_span("a\nb\n", "A\nb\n").start, 1);
        let last = edit_span("a\nb\n", "a\nB\n");
        assert_eq!((last.start, last.end), (2, 2));
        // Whole-file replacement.
        let span = edit_span("a\nb\n", "x\ny\nz\n");
        assert_eq!((span.start, span.end), (1, 3));
    }

    /// A pure deletion still names the line it was removed from.
    #[test]
    fn a_deletion_reports_the_place_it_was_removed_from() {
        let span = edit_span("a\nb\nc\n", "a\nc\n");
        assert_eq!(span.start, 2);
    }

    fn edit_sides(before: &str, after: &str) -> (String, String) {
        match file_edit("f".into(), before, after, None) {
            noob_proto::Event::FileEdit { before, after, .. } => (before, after),
            other => panic!("expected a file.edit frame, got {other:?}"),
        }
    }

    /// A one-line fix in a large file must send two lines, not the file. This
    /// is the difference between a diff view that keeps up with the agent and
    /// one that resends everything on every keystroke-sized change.
    #[test]
    fn a_file_edit_carries_only_the_changed_middle() {
        let before: String = (1..=200).map(|n| format!("line {n}\n")).collect();
        let after = before.replace("line 100\n", "LINE 100\n");
        let (old, new) = edit_sides(&before, &after);
        assert_eq!(old, "line 100");
        assert_eq!(new, "LINE 100");
    }

    #[test]
    fn a_file_edit_carries_both_sides_of_an_insertion_and_a_deletion() {
        // Insertion: nothing on the old side, the new lines on the new side.
        let (old, new) = edit_sides("a\nc\n", "a\nb1\nb2\nc\n");
        assert_eq!(old, "");
        assert_eq!(new, "b1\nb2");
        // Deletion: the reverse.
        let (old, new) = edit_sides("a\nb\nc\n", "a\nc\n");
        assert_eq!(old, "b");
        assert_eq!(new, "");
        // A new file is entirely new.
        let (old, new) = edit_sides("", "x\ny\n");
        assert_eq!(old, "");
        assert_eq!(new, "x\ny");
        // Nothing changed at all: an empty region, not a whole-file resend.
        let (old, new) = edit_sides("a\nb\n", "a\nb\n");
        assert_eq!((old.as_str(), new.as_str()), ("", ""));
    }

    /// Scattered changes collapse into one region spanning them. Coarse on
    /// purpose: the frame stays one frame, and the span still bounds where a
    /// consumer has to look.
    #[test]
    fn scattered_changes_report_one_region_that_covers_them() {
        let (old, new) = edit_sides("a\nb\nc\nd\ne\n", "a\nB\nc\nD\ne\n");
        assert_eq!(old, "b\nc\nd");
        assert_eq!(new, "B\nc\nD");
    }
}
