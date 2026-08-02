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

## Steps

Each ends green (`./dev.sh gui-test`, `./dev.sh test`) and committed on its own.

1. **Dead paths out.** The top-level `Reading`/`Field`/`Setting` arms in
   `settings/paint.rs` and their placement in `settings/places.rs`, the two
   helpers that only fed them, and `SETTING_VALUE_COLUMNS`. The popup's margin
   becomes one constant owned by the popup. *Done: -258 lines.*
2. **View gives back what is not its own.** Picker metrics to `picker/`, gauge
   and context block sizes to those widgets, `Act` to `settings/`. Moves only.
3. **Tests to the box they prove.** A shared `#[cfg(test)]` rig, then the 217
   tests distributed. Duplicates cut on the way through.
4. **`view/mod.rs` into files**: layout, hit testing, chrome, the drawing
   vocabulary. Same box, no new contracts.
5. **`settings/places.rs` + `paint.rs` into one file per row kind**: card,
   table, paper, entry, palette. A kind's geometry and its paint side by side.
6. **`main.rs` into files** by job: input routing, deeds, the process bridges.
   `main.rs` keeps the entry point and the App.
7. **CLI**: `ui/dock.rs` (2,870) and `agent/mod.rs` into files by job.
8. **Contracts**: fix the drift the audit found (`agent::OWNED` says 6 and is 8,
   `commands::ALL` says 28 and is 30, four test counts), and repoint
   `docs/INDEX.md`.

## Left alone, on purpose

- `settings/sections/*` and `widgets/*` are already one folder per display and
  per pane, each with its contract, each 42-322 lines. Nothing to split.
- `crates/noob/src/tools/outline.rs` (627 lines) is unreferenced but its header
  says it is parked deliberately for the harness. Flagged, not deleted.
