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

case "$(uname -m)" in
    x86_64|amd64) docker_arch=amd64 ;;
    aarch64|arm64) docker_arch=arm64 ;;
    *) echo "install.sh: unsupported host architecture: $(uname -m)" >&2; exit 2 ;;
esac

destination="$prefix/bin/noob"
if [[ -e "$destination" && $force -ne 1 ]]; then
    if ! grep -q "noob-cli managed Docker launcher" "$destination" 2>/dev/null; then
        echo "install.sh: refusing to replace unmanaged $destination; pass --force" >&2
        exit 1
    fi
fi

echo "Building isolated runtime image noob:local..."
docker build --build-arg "TARGETARCH=$docker_arch" --target runtime --tag noob:local \
    --file "$ROOT/docker/Dockerfile" "$ROOT"

install -d "$prefix/bin"
install -m 0755 "$ROOT/scripts/noob" "$destination"

config_home="${NOOB_CONFIG_HOME:-${XDG_CONFIG_HOME:-$HOME/.config}/noob}"
install -d "$config_home/skills"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        return 1
    fi
}

# noob 0.5.1 and earlier seeded this exact file. websearch-skill 0.3.0
# removed its MCP server, so leaving the managed entry behind makes every
# attempted connection run a command that no longer exists. Only remove the
# byte-exact seeded file; a user-edited MCP configuration remains theirs.
legacy_websearch_mcp_sha256=a833367a645cba67541f308395eccbc15c8c21aa70c0af732d5b9f2b2aa17808
mcp_config="$config_home/mcp.json"
if [[ -f "$mcp_config" ]] \
    && [[ "$(sha256_file "$mcp_config" 2>/dev/null || true)" == "$legacy_websearch_mcp_sha256" ]]; then
    rm -f -- "$mcp_config"
    echo "Removed obsolete managed websearch MCP configuration."
fi

if [[ -d "$ROOT/config/skills" ]]; then
    for skill in "$ROOT"/config/skills/*; do
        [[ -d "$skill" ]] || continue
        target="$config_home/skills/$(basename "$skill")"
        if [[ ! -e "$target" ]]; then
            cp -R "$skill" "$target"
        elif [[ "$(basename "$skill")" == web-search && -f "$target/SKILL.md" ]]; then
            # Upgrade only a copy noob itself seeded, identified byte for byte.
            # Every previously shipped version is listed, so a user who skips
            # releases still gets the current skill; a copy they edited matches
            # none of these and stays theirs.
            managed_websearch_skill_sha256=(
                29cedb3972661289f254f9711fa90ea983e8b8fb504de35f51c5a27e7cbf9859  # 0.2.6 era
                c74a632085bc1a7e093cd0608cf431ec5f110af094433e565a341ced34f77441  # 0.3.0 era
            )
            current="$(sha256_file "$target/SKILL.md" 2>/dev/null || true)"
            for known in "${managed_websearch_skill_sha256[@]}"; do
                if [[ "$current" == "$known" ]]; then
                    install -m 0644 "$skill/SKILL.md" "$target/SKILL.md"
                    echo "Updated the managed web-search skill for the CLI tool."
                    break
                fi
            done
        fi
    done
fi

echo "Installed $destination"
echo "For scratch work: mkdir -p noob-workspace && cd noob-workspace && noob"
echo "For a project: cd /path/to/project && noob"
echo "Resume: noob --resume <session>"
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Add $prefix/bin to PATH to use the noob command." ;;
esac
