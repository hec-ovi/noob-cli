//! Streaming head+tail retention for a child's merged output. Producers can
//! emit without a size limit; only the bytes that can appear in the final
//! bounded result are kept.

/// Byte-oriented because process output is not guaranteed to be UTF-8.
#[derive(Debug)]
pub(crate) struct HeadTailBuffer {
    head_cap: usize,
    tail_cap: usize,
    head: Vec<u8>,
    tail: Vec<u8>,
    total: usize,
}

impl HeadTailBuffer {
    pub fn new(head_cap: usize, tail_cap: usize) -> Self {
        Self {
            head_cap,
            tail_cap,
            // Uncapped mode passes usize::MAX; pre-allocate only up to the
            // shipped bash budget and let the vectors grow with real input.
            head: Vec::with_capacity(head_cap.min(PREALLOC_HEAD)),
            tail: Vec::with_capacity(tail_cap.min(PREALLOC_TAIL)),
            total: 0,
        }
    }

    /// Retain the configured head and rolling tail while counting every
    /// drained byte for the omission marker.
    pub fn extend(&mut self, mut bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len());

        if self.head.len() < self.head_cap {
            let take = bytes.len().min(self.head_cap - self.head.len());
            self.head.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
        }
        if bytes.is_empty() || self.tail_cap == 0 {
            return;
        }
        if bytes.len() >= self.tail_cap {
            self.tail.clear();
            self.tail
                .extend_from_slice(&bytes[bytes.len() - self.tail_cap..]);
            return;
        }
        let overflow = self
            .tail
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(self.tail_cap);
        if overflow > 0 {
            self.tail.copy_within(overflow.., 0);
            self.tail.truncate(self.tail.len() - overflow);
        }
        self.tail.extend_from_slice(bytes);
    }

    pub fn render(&self) -> String {
        self.render_with("narrow the command if you need the omitted part")
    }

    pub fn render_with(&self, next_action: &str) -> String {
        let kept = self.head.len() + self.tail.len();
        if self.total <= kept {
            let mut all = Vec::with_capacity(kept);
            all.extend_from_slice(&self.head);
            all.extend_from_slice(&self.tail);
            return String::from_utf8_lossy(&all).into_owned();
        }
        let omitted = self.total - kept;
        format!(
            "{}\n[output truncated: {omitted} bytes omitted from the middle; the start and \
             end are shown; {next_action}]\n{}",
            String::from_utf8_lossy(&self.head),
            String::from_utf8_lossy(&self.tail)
        )
    }

    #[cfg(test)]
    fn stored_len(&self) -> usize {
        self.head.len() + self.tail.len()
    }
}

/// The shipped bash budget, so the common case allocates once.
const PREALLOC_HEAD: usize = 8 * 1024;
const PREALLOC_TAIL: usize = 16 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_head_tail_is_bounded_while_draining_unlimited_input() {
        let mut out = HeadTailBuffer::new(10, 20);
        for _ in 0..1_000 {
            out.extend(b"abcdefghijklmnopqrstuvwxyz");
            assert!(out.stored_len() <= 30);
        }
        let rendered = out.render();
        assert!(
            rendered.starts_with("abcdefghij\n[output truncated:"),
            "{rendered}"
        );
        assert!(rendered.ends_with("ghijklmnopqrstuvwxyz"), "{rendered}");
        assert!(rendered.contains("25970 bytes omitted"), "{rendered}");
    }

    #[test]
    fn streaming_buffer_preserves_untruncated_utf8_across_the_split() {
        let mut out = HeadTailBuffer::new(2, 8);
        out.extend("aéz".as_bytes());
        assert_eq!(out.render(), "aéz");
    }

    #[test]
    fn uncapped_streaming_buffer_retains_everything() {
        let mut out = HeadTailBuffer::new(usize::MAX, usize::MAX);
        for _ in 0..1_000 {
            out.extend(b"abcdefghijklmnopqrstuvwxyz");
        }
        let rendered = out.render();
        assert_eq!(rendered.len(), 26_000);
        assert!(!rendered.contains("[output truncated:"));
    }
}
