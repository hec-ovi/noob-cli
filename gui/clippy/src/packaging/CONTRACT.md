# packaging

contractVersion: 1.0.0

## Purpose

Desktop integration: the app id, the activation token, the desktop entry
and icons, and the installer that stages them. Freedesktop today; another
platform's integration is another implementation of this box.

## Public surface

The app id and window-class naming, activation-token handling for the
window, and the install/uninstall staging of `gui/data/` (the desktop
entry and hicolor icons) under the user's prefix.

## Invariants

1. Install and package stage the same one directory, so the two can never
   disagree about what ships.
2. Uninstall removes exactly what install staged, nothing else.
3. Everything is per-user: no root, no system paths.

## Dependencies

`gui/data/` (the desktop entry, icons, the data install script);
`dev.sh gui-package` drives it.

## Tests

Inline: the installer runs against a scratch prefix and asserts the staged
tree.
