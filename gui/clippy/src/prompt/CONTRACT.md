# prompt

contractVersion: 1.0.0

## Purpose

The line you type: text, caret, and selection, counted in characters so a
caret can never land inside a multibyte character.

## Public surface

```rust
pub struct Prompt;
impl Prompt {
    pub fn text(&self) -> &str;
    pub fn caret(&self) -> usize;          // chars, 0..=len
    pub fn len(&self) -> usize;  pub fn is_empty(&self) -> bool;
    pub fn selection(&self) -> Option<(usize, usize)>;   // ordered span
    pub fn selected(&self) -> Option<String>;
    pub fn select_all(&mut self);
    pub fn place(&mut self, at: usize);    // caret only, drops the anchor
    pub fn press(&mut self, at: usize);    // anchor here
    pub fn drag_to(&mut self, at: usize);  // extend from the anchor
    pub fn insert(&mut self, typed: &str); // replaces a selection
    pub fn backspace(&mut self) -> bool;   // true when something changed
    pub fn delete(&mut self) -> bool;
}
```

## Invariants

1. Positions are characters, never bytes: backspace removes one whole
   character, whatever the platform's composition produced.
2. A selection is anchor plus caret, so backwards and forwards drags are
   the same span.
3. Insert over a selection replaces it atomically.

## Dependencies

None. Pure model; the shell routes keys, the view paints.

## Tests

Inline: caret motion, span ordering, multibyte edits (13 tests).
