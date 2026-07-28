//! The animated ASCII avatar, and the file format it plays.
//!
//! Nothing here decodes an image. `gui/asciify` turns a GIF into a text file
//! once, at authoring time, and this reads that file: a decoder shipped to
//! every user to read one file that never changes is a decoder nobody needed.
//!
//! ```text
//! avatar 1 40x19      # version, then the grid every frame is drawn on
//! f 100               # a frame, and how long it holds, in milliseconds
//! ...19 lines...
//! f 100
//! ...
//! ```
//!
//! Trailing spaces are stripped by the converter, so a line may be shorter
//! than the grid and an empty line is a blank row. Reading it back does not
//! pad: the renderer draws text, and a run of spaces at the end of a line is
//! the same as nothing there.
//!
//! One frame is held for its own delay rather than the whole animation running
//! at a fixed rate, because a GIF's timing is part of what it is: the pause at
//! the end of a loop is what makes a loop read as a loop.

/// The clip that ships. Small enough to embed (about 35 KiB of text) and it
/// means the view works before anyone has configured anything.
const BUILT_IN: &str = include_str!("../avatar/clippy.txt");

pub struct Avatar {
    frames: Vec<Frame>,
    /// Every frame's delay summed, so the position in the loop is one modulo
    /// rather than a walk.
    total_ms: u64,
    pub cols: usize,
    /// The grid the file declared. Read when parsing, to bound how many lines
    /// a frame may claim, and asserted against in the tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub rows: usize,
}

struct Frame {
    /// Milliseconds from the start of the loop at which this frame stops
    /// showing. Cumulative, so finding the current frame is a binary search
    /// instead of a subtraction loop.
    until_ms: u64,
    lines: Vec<String>,
}

impl Avatar {
    /// The clip built into this binary.
    pub fn built_in() -> Option<Avatar> {
        Avatar::parse(BUILT_IN)
    }

    /// A clip from a file the settings named. None when it cannot be read or
    /// is not one of these, so a bad path costs the avatar and not the window.
    pub fn load(path: &std::path::Path) -> Option<Avatar> {
        Avatar::parse(&std::fs::read_to_string(path).ok()?)
    }

    pub fn parse(text: &str) -> Option<Avatar> {
        let mut lines = text.lines();
        let header = lines.next()?;
        let mut header = header.split_whitespace();
        if header.next()? != "avatar" {
            return None;
        }
        // A version this build does not know is not a clip it can draw. Better
        // no avatar than a misread one.
        if header.next()? != "1" {
            return None;
        }
        let (cols, rows) = header.next()?.split_once('x')?;
        let (cols, rows): (usize, usize) = (cols.parse().ok()?, rows.parse().ok()?);
        if cols == 0 || rows == 0 {
            return None;
        }

        let mut frames: Vec<Frame> = Vec::new();
        let mut elapsed = 0u64;
        let mut pending: Option<u64> = None;
        let mut body: Vec<String> = Vec::new();
        let finish = |delay: u64, body: &mut Vec<String>, elapsed: &mut u64, out: &mut Vec<Frame>| {
            *elapsed += delay;
            out.push(Frame {
                until_ms: *elapsed,
                lines: std::mem::take(body),
            });
        };
        for line in lines {
            if let Some(rest) = line.strip_prefix("f ") {
                if let Some(delay) = pending.take() {
                    finish(delay, &mut body, &mut elapsed, &mut frames);
                }
                // A delay of zero would make a frame that is never shown and,
                // for a whole clip of them, a division by zero at play time.
                pending = Some(rest.trim().parse::<u64>().ok()?.max(1));
                continue;
            }
            if pending.is_some() && body.len() < rows {
                body.push(line.to_string());
            }
        }
        if let Some(delay) = pending.take() {
            finish(delay, &mut body, &mut elapsed, &mut frames);
        }
        if frames.is_empty() || elapsed == 0 {
            return None;
        }
        Some(Avatar {
            frames,
            total_ms: elapsed,
            cols,
            rows,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    /// The frame showing `at_ms` into the animation, looping.
    pub fn frame_at(&self, at_ms: u64) -> &[String] {
        let position = at_ms % self.total_ms;
        let index = self
            .frames
            .partition_point(|frame| frame.until_ms <= position)
            .min(self.frames.len() - 1);
        &self.frames[index].lines
    }

    /// Milliseconds until the frame showing at `at_ms` is replaced.
    ///
    /// The window sleeps exactly this long rather than waking on a timer of its
    /// own: an avatar that holds a frame for a second must not cost sixty
    /// redraws to do it.
    pub fn hold_ms(&self, at_ms: u64) -> u64 {
        let position = at_ms % self.total_ms;
        let index = self
            .frames
            .partition_point(|frame| frame.until_ms <= position)
            .min(self.frames.len() - 1);
        self.frames[index].until_ms.saturating_sub(position).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIP: &str = "avatar 1 4x2\nf 50\nab\ncd\nf 30\nef\ngh\n";

    #[test]
    fn a_clip_reads_back_as_its_frames() {
        let avatar = Avatar::parse(CLIP).unwrap();
        assert_eq!((avatar.cols, avatar.rows), (4, 2));
        assert_eq!(avatar.frames(), 2);
        assert_eq!(avatar.frame_at(0), ["ab", "cd"]);
        assert_eq!(avatar.frame_at(49), ["ab", "cd"]);
        assert_eq!(avatar.frame_at(50), ["ef", "gh"]);
        assert_eq!(avatar.frame_at(79), ["ef", "gh"]);
    }

    /// An animation is a loop, and the loop is what makes it one.
    #[test]
    fn time_past_the_end_wraps_to_the_start() {
        let avatar = Avatar::parse(CLIP).unwrap();
        assert_eq!(avatar.frame_at(80), ["ab", "cd"]);
        assert_eq!(avatar.frame_at(80 * 1000 + 55), ["ef", "gh"]);
    }

    /// The window sleeps for exactly one frame rather than polling: an avatar
    /// that holds a frame for a second must not cost sixty redraws.
    #[test]
    fn the_hold_is_the_time_left_on_this_frame() {
        let avatar = Avatar::parse(CLIP).unwrap();
        assert_eq!(avatar.hold_ms(0), 50);
        assert_eq!(avatar.hold_ms(49), 1);
        assert_eq!(avatar.hold_ms(50), 30);
        assert_eq!(avatar.hold_ms(79), 1);
        assert!(avatar.hold_ms(80) > 0, "and never zero at the wrap");
    }

    /// Anything that is not a clip this build can draw is no avatar at all,
    /// rather than a misread one.
    #[test]
    fn nonsense_is_refused_rather_than_half_read() {
        assert!(Avatar::parse("").is_none());
        assert!(Avatar::parse("not an avatar\n").is_none());
        assert!(Avatar::parse("avatar 9 4x2\nf 50\nab\ncd\n").is_none(), "a newer version");
        assert!(Avatar::parse("avatar 1 0x0\nf 50\n").is_none());
        assert!(Avatar::parse("avatar 1 4x2\n").is_none(), "no frames");
        assert!(Avatar::parse("avatar 1 4x2\nf zero\nab\n").is_none());
    }

    /// A frame that holds for no time would never be seen, and a clip of them
    /// would divide by zero at play time.
    #[test]
    fn a_frame_always_holds_for_some_time() {
        let avatar = Avatar::parse("avatar 1 2x1\nf 0\nxy\n").unwrap();
        assert_eq!(avatar.frame_at(0), ["xy"]);
        assert!(avatar.hold_ms(0) >= 1);
    }

    /// Trailing spaces are stripped by the converter, so a short line and an
    /// empty one are both ordinary.
    #[test]
    fn short_and_empty_lines_are_kept_as_written() {
        let avatar = Avatar::parse("avatar 1 6x3\nf 10\nab\n\ncdef\n").unwrap();
        assert_eq!(avatar.frame_at(0), ["ab", "", "cdef"]);
    }

    /// The clip that ships has to be one this build can actually play, or the
    /// view is empty for everyone who never configured anything.
    #[test]
    fn the_built_in_clip_parses_and_animates() {
        let avatar = Avatar::built_in().expect("the embedded clip is playable");
        assert!(avatar.frames() > 10, "{} frames", avatar.frames());
        assert!(avatar.cols > 10 && avatar.rows > 5);
        // It actually moves: two frames far apart are not the same picture.
        let first = avatar.frame_at(0).to_vec();
        let later = avatar.frame_at(avatar.total_ms / 2).to_vec();
        assert_ne!(first, later);
        // Every frame fits the grid it declared.
        for frame in &avatar.frames {
            assert!(frame.lines.len() <= avatar.rows);
            for line in &frame.lines {
                assert!(line.chars().count() <= avatar.cols, "{line:?}");
            }
        }
    }
}
