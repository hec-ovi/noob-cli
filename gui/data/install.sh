#!/usr/bin/env bash
# Install CLIppy for the current user. No root, nothing outside $HOME.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
APP_ID="io.github.hec_ovi.CLIppy"

prefix="${CLIPPY_INSTALL_PREFIX:-$HOME/.local}"
uninstall=0

usage() {
    cat <<'EOF'
usage: ./install.sh [--prefix <dir>] [--uninstall]

Installs <prefix>/bin/clippy, the desktop entry and the icons.
The default prefix is $CLIPPY_INSTALL_PREFIX or ~/.local, so nothing needs root
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

if ((uninstall)); then
    rm -f -- "$prefix/bin/clippy" \
             "$apps/$APP_ID.desktop" \
             "$scalable/$APP_ID.svg" \
             "$symbolic/$APP_ID-symbolic.svg"
    echo "Removed CLIppy from $prefix."
else
    install -d "$prefix/bin" "$apps" "$scalable" "$symbolic"
    install -m 0755 "$HERE/clippy" "$prefix/bin/clippy"
    install -m 0644 "$HERE/$APP_ID.desktop" "$apps/$APP_ID.desktop"
    install -m 0644 "$HERE/$APP_ID.svg" "$scalable/$APP_ID.svg"
    install -m 0644 "$HERE/$APP_ID-symbolic.svg" "$symbolic/$APP_ID-symbolic.svg"
    echo "Installed $prefix/bin/clippy"
fi

# Desktops cache what is in these directories and will not notice a change
# until told. Both commands are optional; a desktop without them picks the
# change up on the next login instead.
command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database -q "$apps" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && gtk-update-icon-cache -qtf "$prefix/share/icons/hicolor" 2>/dev/null || true

((uninstall)) && exit 0

cat <<EOF

CLIppy needs the agent on PATH. Install it with the repository's own
install.sh, or point CLIppy at a build with NOOB_BIN=/path/to/noob.

Run it from a project directory:  clippy
Or name one:                      clippy ~/some/project
EOF

case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Add $prefix/bin to PATH to run clippy from a shell." ;;
esac
