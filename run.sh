#!/data/data/com.termux/files/usr/bin/bash
# Build and launch Scamper. Pass-through args go to the game binary, so:
#   ./run.sh                 # play (the box-arena test app)
#   ./run.sh verify ./scratch  # headless: scripted scenarios + PNG dumps
#   ./run.sh info            # print the detected terminal size and exit
#   ./run.sh sprites         # sprite-lab: view sprite animations
#
# Needs a terminal speaking the Kitty graphics + keyboard protocols
# (Kitty, Ghostty, or foot). Not tmux, not Konsole. (The text backends and the
# sprite-lab run in any terminal.)
set -euo pipefail

cd "$(dirname "$0")"

# Debug build is faster to compile; release is smoother to play. Default release;
# `SCAMP_DEBUG=1 ./run.sh` swaps to a quick debug build for iteration.
profile_args=(--release)
profile_dir=release
if [[ "${SCAMP_DEBUG:-0}" == "1" ]]; then
    profile_args=()
    profile_dir=debug
fi

# `./run.sh sprites` launches the standalone sprite-lab tool instead of the game.
if [[ "${1:-}" == "sprites" || "${1:-}" == "lab" || "${1:-}" == "sprite-lab" ]]; then
    cargo build "${profile_args[@]}" --bin sprite-lab
    exec "target/$profile_dir/sprite-lab"
fi

cargo build "${profile_args[@]}" --bin scamp

# A heads-up (non-fatal) if we're somewhere graphics won't work.
if [[ -n "${TMUX:-}" ]]; then
    echo "warning: running under tmux — Kitty graphics won't pass through." >&2
fi

# Default to --debug logging (to ./scamp.log) during development. Drop this once
# we cut a release. Skip auto-adding it if the caller already passed --debug.
extra=()
if [[ "${SCAMP_NODEBUG:-0}" != "1" ]] && [[ ! " $* " == *" --debug "* ]]; then
    extra+=(--debug)
fi

exec "target/$profile_dir/scamp" "$@" "${extra[@]}"
