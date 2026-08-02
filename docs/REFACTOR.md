# Refactor: lean the tree

One rule: **move and delete, never multiply.** A step either moves code to where
it belongs or deletes code nothing needs. No step adds a layer, a wrapper or a
contract for something that is a file.

A new `CONTRACT.md` only where a folder is a box outsiders consume. Row kinds,
chrome painters and window jobs are files inside the box they already belong to.

## What the audit found

- `view/mod.rs` is 17,438 lines: 4,028 of code and 13,410 holding 217 tests that
  prove nine widgets, the settings panel, the picker, the menu and the dock.
- `main.rs` is 7,660: 5,234 of code, the rest its own tests.
- `settings/` places and paints every row kind in two shared files.
- Foreign vocabulary lives in `view/`: the picker's whole metric set (181
  lines), the settings `Act` enum, the gauge and context block sizes.
- Dead paths: three `Row` kinds placed and painted at top level that no section
  ever builds; a second constant for the popup's margin.

## What was done

Each step ended green (`./dev.sh gui-test`, `./dev.sh test`) and was committed on
its own.

1. *Done.* **Dead paths out.** The top-level `Reading`/`Field`/`Setting` arms in
   `settings/paint.rs` and their placement in `settings/places.rs`, the two
   helpers that only fed them, and `SETTING_VALUE_COLUMNS`. The popup's margin
   becomes one constant owned by the popup. *Done: -258 lines.*
2. *Done.* **View gives back what is not its own.** Picker metrics to `picker/`, gauge
   and context block sizes to those widgets, `Act` to `settings/`. Moves only.
3. *Done.* **Tests to the box they prove.** A shared `#[cfg(test)]` rig
   (`view/testkit.rs`), then 217 tests distributed: `view/mod.rs` 17,438 ->
   6,517.
4. *Done.* **`view/mod.rs` into files**: layout, hit testing, chrome, the drawing
   vocabulary. Same box, no new contracts.
5. *Not done.* **`settings/places.rs` + `paint.rs` per row kind**: card,
   table, paper, entry, palette. A kind's geometry and its paint side by side.
6. *Done, in part.* **`main.rs` into files**: its tests are `tests.rs` and its
   free helpers are `shell.rs`; `impl App` is still one block.
7. *Done, in part.* **CLI**: the dock's pinned rows are `ui/regions.rs`; `ui/dock.rs` is 2,515 and `agent/mod.rs` untouched.
8. *Done.* **Contracts**: fix the drift the audit found (`agent::OWNED` says 6 and is 8,
   `commands::ALL` says 28 and is 30, four test counts), and repoint
   `docs/INDEX.md`.

## Left alone, on purpose

- `settings/sections/*` and `widgets/*` are already one folder per display and
  per pane, each with its contract, each 42-322 lines. Nothing to split.
- `crates/noob/src/tools/outline.rs` was 627 unreferenced lines parked for a
  harness that never called it. Deleted.

## What is left

- `settings/places.rs` (1,782) and `paint.rs` (2,600 with its scene tests) still
  place and paint every row kind in two files. The cut is per row kind, not per
  section: a card is used by five sections and a note by six, so per-section
  files would copy the card painter five times.
- `main.rs` `impl App` is 3,400 lines in one block: the event loop, the input
  routing, the deeds and the process bridges.
- `crates/noob/src/agent/mod.rs` (2,873) and `ui/dock.rs` (2,515).
