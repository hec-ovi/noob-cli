#!/usr/bin/env bash
# Install NO0B for the current user. No root, nothing outside $HOME.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
APP_ID="io.github.hec_ovi.NO0B"
# The window shipped as CLIppy up to 0.6.0. Its launcher, entry and icons are
# removed by every route below, install as well as uninstall: without that the
# rename leaves two launchers in the menu, one of them pointing at a binary that
# is no longer written.
OLD_APP_ID="io.github.hec_ovi.CLIppy"
OLD_BIN="clippy"

prefix="${NO0B_INSTALL_PREFIX:-$HOME/.local}"
uninstall=0

usage() {
    cat <<'EOF'
usage: ./install.sh [--prefix <dir>] [--uninstall]

Installs <prefix>/bin/no0b, the desktop entry and the icons.
The default prefix is $NO0B_INSTALL_PREFIX or ~/.local, so nothing needs root
and nothing lands outside your home directory.
EOF
}

while (($#)); do
    case "$1" in
        --prefix)
            [[ $# -ge 2 ]] || { echo "install.sh: --prefix needs a directory" >&2; exit 2; }
            prefix="$2"
            shift 2
            ;;
        --uninstall)
            uninstall=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "install.sh: unknown option $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

apps="$prefix/share/applications"
scalable="$prefix/share/icons/hicolor/scalable/apps"
symbolic="$prefix/share/icons/hicolor/symbolic/apps"

# Whatever the old name left behind. `rm -f` throughout, so a machine that never
# had CLIppy installed removes nothing and reports nothing.
remove_old() {
    rm -f -- "$prefix/bin/$OLD_BIN" \
             "$apps/$OLD_APP_ID.desktop" \
             "$scalable/$OLD_APP_ID.svg" \
             "$symbolic/$OLD_APP_ID-symbolic.svg"
}

if ((uninstall)); then
    rm -f -- "$prefix/bin/no0b" \
             "$apps/$APP_ID.desktop" \
             "$scalable/$APP_ID.svg" \
             "$symbolic/$APP_ID-symbolic.svg"
    remove_old
    echo "Removed NO0B from $prefix."
else
    install -d "$prefix/bin" "$apps" "$scalable" "$symbolic"
    install -m 0755 "$HERE/no0b" "$prefix/bin/no0b"
    install -m 0644 "$HERE/$APP_ID.desktop" "$apps/$APP_ID.desktop"
    # The launcher runs the binary by its full path rather than by name. A
    # desktop session does not necessarily have ~/.local/bin on its PATH even
    # when your shell does, and the failure is the worst kind: the icon is
    # there, clicking it does nothing, and nothing anywhere says why.
    sed -i "s|^Exec=no0b|Exec=$prefix/bin/no0b|" "$apps/$APP_ID.desktop"
    install -m 0644 "$HERE/$APP_ID.svg" "$scalable/$APP_ID.svg"
    install -m 0644 "$HERE/$APP_ID-symbolic.svg" "$symbolic/$APP_ID-symbolic.svg"
    remove_old
    echo "Installed $prefix/bin/no0b"
fi

# Desktops cache what is in these directories and will not notice a change
# until told. Both commands are optional; a desktop without them picks the
# change up on the next login instead.
command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database -q "$apps" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && gtk-update-icon-cache -qtf "$prefix/share/icons/hicolor" 2>/dev/null || true

((uninstall)) && exit 0

# Only say this when it is true. Advice about a thing you already have is
# noise, and noise is how output stops being read.
if ! command -v "${NOOB_BIN:-noob}" >/dev/null 2>&1; then
    cat <<'EOF'

The agent itself is not on PATH. Install it with the repository's own
install.sh, or point NO0B at a build with NOOB_BIN=/path/to/noob.
EOF
fi

cat <<'EOF'

Run it from a project directory:  no0b
Or name one:                      no0b ~/some/project
EOF

case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Add $prefix/bin to PATH to run no0b from a shell." ;;
esac
