//! Byte-exact incremental SSE parser. The only SSE parser in the binary:
//! it serves both wire adapters now and MCP Streamable HTTP in P4.
//!
//! Fed raw TCP bytes in whatever chunks arrive; emits complete events.
//! Handles every split the network can produce: chunk boundaries inside a
//! line, inside a `data:` keyword, or inside a multibyte UTF-8 codepoint
//! (line terminators are ASCII, so buffering raw bytes until a terminator
//! makes codepoint splits structurally safe). Tolerates CRLF, LF and bare
//! CR, a BOM on the first line, comment keepalives (`: OPENROUTER
//! PROCESSING`), optional space after the colon, and `id:`/`retry:` fields
//! (dropped). `event:` is captured; the Responses adapter routes on it.

/// One dispatched server-sent event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field, if the server sent one for this event.
    pub event: Option<String>,
    /// All `data:` lines joined with `\n`.
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    /// Bytes of the current, not-yet-terminated line.
    line: Vec<u8>,
    /// Last byte seen was CR: swallow an immediately following LF.
    swallow_lf: bool,
    /// True until the first line is processed (BOM strip window).
    at_start: bool,
    event: Option<String>,
    data: String,
    has_data: bool,
}

impl SseParser {
    pub fn new() -> SseParser {
        SseParser {
            at_start: true,
            ..Default::default()
        }
    }

    /// Feed one chunk of raw bytes; push every completed event onto `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<SseEvent>) {
        for &b in bytes {
            if self.swallow_lf {
                self.swallow_lf = false;
                if b == b'\n' {
                    continue;
                }
            }
            match b {
                b'\r' => {
                    self.swallow_lf = true;
                    self.end_line(out);
                }
                b'\n' => self.end_line(out),
                _ => self.line.push(b),
            }
        }
    }

    /// Signal end of stream. A complete (terminated) trailing event without
    /// its final blank line is dispatched: several backends end streams
    /// without `[DONE]` or the closing separator. Unterminated trailing
    /// bytes are truncation and are dropped.
    pub fn finish(&mut self, out: &mut Vec<SseEvent>) {
        if self.line.is_empty() {
            self.dispatch(out);
        }
        self.event = None;
        self.data.clear();
        self.has_data = false;
    }

    fn end_line(&mut self, out: &mut Vec<SseEvent>) {
        let mut line = std::mem::take(&mut self.line);
        if self.at_start {
            self.at_start = false;
            if line.starts_with(&[0xEF, 0xBB, 0xBF]) {
                line.drain(..3);
            }
        }
        if line.is_empty() {
            self.dispatch(out);
            return;
        }
        if line[0] == b':' {
            return; // comment / keepalive
        }
        let (name, value) = match line.iter().position(|&b| b == b':') {
            Some(i) => {
                let v = &line[i + 1..];
                let v = v.strip_prefix(b" ").unwrap_or(v);
                (&line[..i], v)
            }
            // A line with no colon is a field name with an empty value.
            None => (&line[..], &[][..]),
        };
        match name {
            b"data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(&String::from_utf8_lossy(value));
                self.has_data = true;
            }
            b"event" => {
                self.event = Some(String::from_utf8_lossy(value).into_owned());
            }
            // id: and retry: are legal SSE we have no use for.
            _ => {}
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        if self.has_data {
            out.push(SseEvent {
                event: self.event.take(),
                data: std::mem::take(&mut self.data),
            });
        } else {
            // Per the SSE spec an empty event resets the type without dispatch.
            self.event = None;
        }
        self.has_data = false;
    }
}

/// Parse a whole buffer in one call (fixtures, tests).
pub fn parse_all(bytes: &[u8]) -> Vec<SseEvent> {
    let mut p = SseParser::new();
    let mut out = Vec::new();
    p.feed(bytes, &mut out);
    p.finish(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(event: Option<&str>, data: &str) -> SseEvent {
        SseEvent {
            event: event.map(str::to_string),
            data: data.to_string(),
        }
    }

    #[test]
    fn basic_event_lf() {
        assert_eq!(parse_all(b"data: hello\n\n"), vec![ev(None, "hello")]);
    }

    #[test]
    fn crlf_and_bare_cr_terminators() {
        assert_eq!(parse_all(b"data: a\r\n\r\n"), vec![ev(None, "a")]);
        assert_eq!(parse_all(b"data: a\r\r"), vec![ev(None, "a")]);
        // Mixed within one stream.
        assert_eq!(
            parse_all(b"data: a\r\n\ndata: b\n\r\n"),
            vec![ev(None, "a"), ev(None, "b")]
        );
    }

    #[test]
    fn multiple_data_lines_join_with_newline() {
        assert_eq!(parse_all(b"data: a\ndata: b\n\n"), vec![ev(None, "a\nb")]);
    }

    #[test]
    fn optional_space_after_colon() {
        assert_eq!(parse_all(b"data:no-space\n\n"), vec![ev(None, "no-space")]);
        // Only ONE leading space is stripped.
        assert_eq!(parse_all(b"data:  two\n\n"), vec![ev(None, " two")]);
    }

    #[test]
    fn empty_data_line_contributes_empty_string() {
        assert_eq!(parse_all(b"data:\ndata: x\n\n"), vec![ev(None, "\nx")]);
        assert_eq!(parse_all(b"data\n\n"), vec![ev(None, "")]);
    }

    #[test]
    fn comment_keepalives_ignored() {
        assert_eq!(
            parse_all(b": OPENROUTER PROCESSING\n\ndata: x\n\n: again\n\n"),
            vec![ev(None, "x")]
        );
    }

    #[test]
    fn bom_stripped_on_first_line_only() {
        assert_eq!(parse_all(b"\xEF\xBB\xBFdata: a\n\n"), vec![ev(None, "a")]);
        // A BOM later in the stream is content, not a BOM.
        let got = parse_all(b"data: a\n\n\xEF\xBB\xBFdata: b\n\n");
        assert_eq!(
            got.len(),
            1,
            "the bogus second line is not a data field: {got:?}"
        );
    }

    #[test]
    fn event_field_captured_and_reset() {
        assert_eq!(
            parse_all(b"event: response.completed\ndata: {}\n\ndata: y\n\n"),
            vec![ev(Some("response.completed"), "{}"), ev(None, "y")]
        );
    }

    #[test]
    fn event_without_data_resets_type_without_dispatch() {
        assert_eq!(
            parse_all(b"event: ping\n\ndata: x\n\n"),
            vec![ev(None, "x")]
        );
    }

    #[test]
    fn id_and_retry_dropped() {
        assert_eq!(
            parse_all(b"id: 7\nretry: 100\ndata: x\n\n"),
            vec![ev(None, "x")]
        );
    }

    #[test]
    fn done_sentinel_is_ordinary_data() {
        assert_eq!(parse_all(b"data: [DONE]\n\n"), vec![ev(None, "[DONE]")]);
    }

    #[test]
    fn finish_flushes_terminated_trailing_event() {
        // Stream ends after a terminated data line but without the blank line.
        assert_eq!(
            parse_all(b"data: a\n\ndata: tail\n"),
            vec![ev(None, "a"), ev(None, "tail")]
        );
    }

    #[test]
    fn finish_drops_unterminated_truncation() {
        // The connection died mid-line: no trailing terminator, not an event.
        assert_eq!(parse_all(b"data: a\n\ndata: {\"trunc"), vec![ev(None, "a")]);
    }

    #[test]
    fn multibyte_codepoint_split_across_feeds() {
        let full = "data: caf\u{00e9} \u{1F980}\n\n".as_bytes();
        // Split inside the 2-byte é and inside the 4-byte crab.
        for cut in 1..full.len() {
            let mut p = SseParser::new();
            let mut out = Vec::new();
            p.feed(&full[..cut], &mut out);
            p.feed(&full[cut..], &mut out);
            p.finish(&mut out);
            assert_eq!(out, vec![ev(None, "caf\u{00e9} \u{1F980}")], "cut at {cut}");
        }
    }

    #[test]
    fn every_two_way_split_of_a_full_transcript_is_identical() {
        let full = b"\xEF\xBB\xBFevent: response.output_text.delta\r\ndata: {\"delta\":\"h\xC3\xA9\"}\r\n\r\n: keepalive\r\ndata: [DONE]\n\n";
        let want = parse_all(full);
        assert_eq!(want.len(), 2);
        for cut in 0..=full.len() {
            let mut p = SseParser::new();
            let mut out = Vec::new();
            p.feed(&full[..cut], &mut out);
            p.feed(&full[cut..], &mut out);
            p.finish(&mut out);
            assert_eq!(out, want, "cut at {cut}");
        }
    }

    #[test]
    fn byte_at_a_time_matches_one_shot() {
        let full = b"data: a\ndata: b\r\nevent: t\rdata: c\n\ndata: d\n\n";
        let want = parse_all(full);
        let mut p = SseParser::new();
        let mut out = Vec::new();
        for &b in full.iter() {
            p.feed(&[b], &mut out);
        }
        p.finish(&mut out);
        assert_eq!(out, want);
    }

    #[test]
    fn crlf_split_between_cr_and_lf_across_feeds() {
        let mut p = SseParser::new();
        let mut out = Vec::new();
        p.feed(b"data: x\r", &mut out);
        p.feed(b"\n\r\n", &mut out);
        p.finish(&mut out);
        assert_eq!(out, vec![ev(None, "x")]);
    }
}
