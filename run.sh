#!/data/data/com.termux/files/usr/bin/bash
# Build and launch Scamper. Pass-through args go to the binary, so:
#   ./run.sh                 # play (the box-arena test app)
#   ./run.sh verify ./scratch  # headless: scripted scenarios + PNG dumps
#   ./run.sh info            # print the detected terminal size and exit
#
# Needs a terminal speaking the Kitty graphics + keyboard protocols
# (Kitty, Ghostty, or foot). Not tmux, not Konsole.
set -euo pipefail

cd "$(dirname "$0")"

# Debug build is faster to compile; release is smoother to play. Default release;
# `SCAMP_DEBUG=1 ./run.sh` swaps to a quick debug build for iteration.
profile_args=(--release)
bin=target/release/scamp
if [[ "${SCAMP_DEBUG:-0}" == "1" ]]; then
    profile_args=()
    bin=target/debug/scamp
fi

cargo build "${profile_args[@]}"

# A heads-up (non-fatal) if we're somewhere graphics won't work.
if [[ -n "${TMUX:-}" ]]; then
    echo "warning: running under tmux — Kitty graphics won't pass through." >&2
fi

exec "$bin" "$@"
