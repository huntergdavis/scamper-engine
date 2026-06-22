#!/usr/bin/env bash
# Install Scamper's toolchain + build the binaries. Works on Linux and Termux.
#
#   ./install.sh            # ensure Rust, then build (release)
#   ./install.sh --debug    # build the faster-compiling debug profile instead
#   ./install.sh --link     # also symlink `scamp` into a bin dir on your PATH
#   ./install.sh --link-dir DIR   # symlink target dir (implies --link)
#
# On Termux, Rust comes from `pkg install rust`. On Linux, if cargo is missing
# we offer to install it via rustup (https://rustup.rs). Either way we finish by
# building the release binaries; play with `./run.sh`.
set -euo pipefail

cd "$(dirname "$0")"

# ---- options -----------------------------------------------------------------
profile_args=(--release)
profile_dir=release
do_link=0
link_dir=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug) profile_args=(); profile_dir=debug ;;
        --release) profile_args=(--release); profile_dir=release ;;
        --link) do_link=1 ;;
        --link-dir) do_link=1; link_dir="${2:-}"; shift ;;
        -h | --help) sed -n '2,12p' "$0"; exit 0 ;;
        *) echo "install.sh: unknown option '$1'" >&2; exit 2 ;;
    esac
    shift
done

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# ---- platform detection ------------------------------------------------------
# Termux exports $PREFIX under its private app dir and reports Android via uname.
is_termux=0
if [[ "${PREFIX:-}" == *com.termux* ]] || [[ "$(uname -o 2>/dev/null)" == "Android" ]]; then
    is_termux=1
fi

# ---- ensure Rust toolchain ---------------------------------------------------
ensure_rust() {
    if command -v cargo >/dev/null 2>&1; then
        say "found cargo: $(cargo --version)"
        return
    fi

    if [[ "$is_termux" == "1" ]]; then
        say "cargo not found — installing Rust via pkg (Termux)"
        pkg install -y rust || die "pkg install rust failed"
    else
        warn "cargo not found."
        if command -v rustup >/dev/null 2>&1; then
            say "rustup is present — installing the stable toolchain"
            rustup default stable
        else
            printf 'Install Rust now via rustup (from https://sh.rustup.rs)? [y/N] '
            read -r reply
            case "$reply" in
                y | Y | yes)
                    command -v curl >/dev/null 2>&1 || die "curl is required to fetch rustup"
                    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
                    # shellcheck disable=SC1090
                    . "${CARGO_HOME:-$HOME/.cargo}/env"
                    ;;
                *) die "Rust is required. Install cargo, then re-run ./install.sh" ;;
            esac
        fi
    fi

    command -v cargo >/dev/null 2>&1 || die "cargo still not on PATH after install"
    say "using cargo: $(cargo --version)"
}

# ---- optional: symlink the game binary onto PATH -----------------------------
link_binary() {
    local src="$PWD/target/$profile_dir/scamp" dest_dir="$link_dir"
    if [[ -z "$dest_dir" ]]; then
        if [[ "$is_termux" == "1" ]]; then
            dest_dir="${PREFIX:-/data/data/com.termux/files/usr}/bin"
        else
            dest_dir="$HOME/.local/bin"
        fi
    fi
    mkdir -p "$dest_dir"
    ln -sf "$src" "$dest_dir/scamp"
    say "linked: $dest_dir/scamp -> $src"
    case ":$PATH:" in
        *":$dest_dir:"*) ;;
        *) warn "$dest_dir is not on your PATH — add it to run \`scamp\` directly." ;;
    esac
}

# ---- run ---------------------------------------------------------------------
ensure_rust

say "building scamp + sprite-lab (${profile_dir})"
cargo build "${profile_args[@]}"

[[ "$do_link" == "1" ]] && link_binary

say "done. Play with:  ./run.sh"
echo "  ./run.sh -i        # interactive menu (game / sprite viewer / tools)"
echo "  ./run.sh verify ./scratch   # headless scenarios + PNG dumps"
