#!/usr/bin/env bash
# Build the isolated runtime image and install the `noob` host command.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
prefix="${NOOB_INSTALL_PREFIX:-$HOME/.local}"
force=0

usage() {
    cat <<'EOF'
usage: ./install.sh [--prefix <dir>] [--force]

Builds the noob:local Docker image and installs <prefix>/bin/noob.
The default prefix is $NOOB_INSTALL_PREFIX or ~/.local.
EOF
}

while (($#)); do
    case "$1" in
        --prefix)
            [[ $# -ge 2 ]] || { echo "install.sh: --prefix needs a directory" >&2; exit 2; }
            prefix="$2"
            shift 2
            ;;
        --force)
            force=1
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

if ! command -v docker >/dev/null 2>&1; then
    echo "install.sh: Docker Engine is required" >&2
    exit 127
fi

destination="$prefix/bin/noob"
if [[ -e "$destination" && $force -ne 1 ]]; then
    if ! grep -q "noob-cli managed Docker launcher" "$destination" 2>/dev/null; then
        echo "install.sh: refusing to replace unmanaged $destination; pass --force" >&2
        exit 1
    fi
fi

echo "Building isolated runtime image noob:local..."
docker build --target runtime --tag noob:local --file "$ROOT/docker/Dockerfile" "$ROOT"

install -d "$prefix/bin"
install -m 0755 "$ROOT/scripts/noob" "$destination"

config_home="${NOOB_CONFIG_HOME:-${XDG_CONFIG_HOME:-$HOME/.config}/noob}"
install -d "$config_home/skills"
if [[ ! -e "$config_home/mcp.json" ]]; then
    install -m 0644 "$ROOT/config/mcp.websearch.example.json" "$config_home/mcp.json"
fi
if [[ -d "$ROOT/config/skills" ]]; then
    for skill in "$ROOT"/config/skills/*; do
        [[ -d "$skill" ]] || continue
        target="$config_home/skills/$(basename "$skill")"
        if [[ ! -e "$target" ]]; then
            cp -R "$skill" "$target"
        fi
    done
fi

echo "Installed $destination"
echo "Run: noob"
echo "Restore: noob --restore <session>"
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Add $prefix/bin to PATH to use the noob command." ;;
esac
