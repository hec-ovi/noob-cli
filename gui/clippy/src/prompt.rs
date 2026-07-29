//! The line you type: its text, its caret, and what is selected in it.
//!
//! Positions are counted in characters, never in bytes. A prompt holds whatever
//! the platform's composition produced, so a caret measured in bytes lands
//! inside an emoji and a backspace removes half of one.
//!
//! A selection is an anchor plus the caret rather than a start and an end, so
//! selecting backwards is the same span as selecting forwards, and every key
//! that moves the caret only has to drop the anchor.

/// What has been typed and where the caret is in it.
#[derive(Debug, Default)]
pub struct Prompt {
    text: String,
    caret: usize,
    /// The fixed end of a selection. `None` when nothing is selected; the
    /// moving end is always the caret.
    anchor: Option<usize>,
}

impl Prompt {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// How many characters long it is, which is also the last caret position.
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The selected span in reading order, or `None` when nothing is selected.
    /// An anchor sitting on the caret is not a selection, the same way a click
    /// that never moved is not one in a pane.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        let (from, to) = (anchor.min(self.caret), anchor.max(self.caret));
        (from < to).then_some((from, to))
    }

    /// What a copy would put on the clipboard.
    pub fn selected(&self) -> Option<String> {
        let (from, to) = self.selection()?;
        Some(self.text.chars().skip(from).take(to - from).collect())
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.caret = self.len();
    }

    /// Put the caret somewhere and drop any selection, which is what a click
    /// in the prompt does.
    pub fn place(&mut self, at: usize) {
        self.caret = at.min(self.len());
        self.anchor = None;
    }

    /// Type over whatever is selected, the way every other text field does.
    pub fn insert(&mut self, typed: &str) {
        self.cut();
        let mut chars: Vec<char> = self.text.chars().collect();
        for (i, c) in typed.chars().enumerate() {
            chars.insert(self.caret + i, c);
        }
        self.caret += typed.chars().count();
        self.text = chars.into_iter().collect();
    }

    /// Returns whether anything changed, so the caller only marks the window
    /// dirty when it has to.
    pub fn backspace(&mut self) -> bool {
        if self.cut() {
            return true;
        }
        if self.caret == 0 {
            return false;
        }
        let mut chars: Vec<char> = self.text.chars().collect();
        chars.remove(self.caret - 1);
        self.text = chars.into_iter().collect();
        self.caret -= 1;
        true
    }

    pub fn delete(&mut self) -> bool {
        if self.cut() {
            return true;
        }
        let mut chars: Vec<char> = self.text.chars().collect();
        if self.caret >= chars.len() {
            return false;
        }
        chars.remove(self.caret);
        self.text = chars.into_iter().collect();
        true
    }

    pub fn left(&mut self) {
        self.place(self.caret.saturating_sub(1));
    }

    pub fn right(&mut self) {
        self.place(self.caret + 1);
    }

    pub fn home(&mut self) {
        self.place(0);
    }

    pub fn end(&mut self) {
        self.place(self.len());
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
        self.anchor = None;
    }

    /// Remove the selection if there is one, and say whether it removed
    /// anything: a key that consumed a selection has already done its job.
    fn cut(&mut self) -> bool {
        let Some((from, to)) = self.selection() else {
            // An anchor that selects nothing still has to go, or the next
            // keystroke would find a stale one and delete from it.
            self.anchor = None;
            return false;
        };
        let chars: Vec<char> = self.text.chars().collect();
        self.text = chars[..from].iter().chain(&chars[to..]).collect();
        self.caret = from;
        self.anchor = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Prompt {
        let mut prompt = Prompt::default();
        prompt.insert(text);
        prompt
    }

    #[test]
    fn select_all_covers_the_whole_line_and_copies_it() {
        let mut prompt = typed("hello world");
        prompt.select_all();
        assert_eq!(prompt.selection(), Some((0, 11)));
        assert_eq!(prompt.selected().as_deref(), Some("hello world"));
        assert_eq!(prompt.caret(), 11);
    }

    /// Nothing typed is nothing selected, and a Ctrl-C on it has to fall
    /// through to whatever else that key does.
    #[test]
    fn select_all_on_an_empty_prompt_selects_nothing() {
        let mut prompt = Prompt::default();
        prompt.select_all();
        assert_eq!(prompt.selection(), None);
        assert_eq!(prompt.selected(), None);
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut prompt = typed("hello world");
        prompt.select_all();
        prompt.insert("bye");
        assert_eq!(prompt.text(), "bye");
        assert_eq!(prompt.caret(), 3);
        assert_eq!(prompt.selection(), None);
    }

    #[test]
    fn backspace_and_delete_remove_the_selection_whole() {
        for remove in [Prompt::backspace, Prompt::delete] {
            let mut prompt = typed("hello world");
            prompt.place(6);
            prompt.select_all();
            assert!(remove(&mut prompt));
            assert_eq!(prompt.text(), "");
            assert_eq!(prompt.caret(), 0);
        }
    }

    /// With nothing selected they still act on one character, and say so only
    /// when there was one to act on.
    #[test]
    fn backspace_and_delete_without_a_selection_take_one_character() {
        let mut prompt = typed("abc");
        prompt.place(1);
        assert!(prompt.backspace());
        assert_eq!(prompt.text(), "bc");
        assert!(!prompt.backspace());
        assert!(prompt.delete());
        assert_eq!(prompt.text(), "c");
        prompt.end();
        assert!(!prompt.delete());
    }

    /// The caret counts characters, so a backspace after an emoji removes the
    /// emoji rather than half of it.
    #[test]
    fn everything_counts_characters_not_bytes() {
        let mut prompt = typed("a\u{1f600}b");
        assert_eq!(prompt.len(), 3);
        prompt.left();
        assert!(prompt.backspace());
        assert_eq!(prompt.text(), "ab");
        prompt.home();
        prompt.right();
        prompt.insert("\u{e9}");
        assert_eq!(prompt.text(), "a\u{e9}b");
    }

    /// An anchor past the caret is the same span as one before it. Nothing
    /// selects backwards yet, and the model is written this way so that the
    /// key which eventually does cannot collapse the selection instead.
    #[test]
    fn an_anchor_after_the_caret_selects_the_same_run() {
        let mut prompt = typed("hello world");
        prompt.select_all();
        let forwards = prompt.selected();
        let mut backwards = typed("hello world");
        backwards.anchor = Some(11);
        backwards.caret = 0;
        assert_eq!(backwards.selection(), Some((0, 11)));
        assert_eq!(backwards.selected(), forwards);
    }

    /// Moving the caret drops the selection, or the next character typed
    /// would delete text nobody is pointing at any more.
    #[test]
    fn moving_the_caret_drops_the_selection() {
        for move_ in [Prompt::left, Prompt::right, Prompt::home, Prompt::end] {
            let mut prompt = typed("hello");
            prompt.select_all();
            move_(&mut prompt);
            assert_eq!(prompt.selection(), None);
            prompt.insert("!");
            assert_eq!(prompt.len(), 6, "{}", prompt.text());
        }
    }

    #[test]
    fn placing_the_caret_clamps_to_the_end_of_the_text() {
        let mut prompt = typed("abc");
        prompt.place(99);
        assert_eq!(prompt.caret(), 3);
        prompt.place(1);
        assert_eq!(prompt.caret(), 1);
    }
}
