#!/data/data/com.termux/files/usr/bin/bash
# Build and launch Scamper.
#   ./run.sh                   # play the game (default)
#   ./run.sh -i                # interactive menu (game / sprite viewer / tools)
#   ./run.sh sprites           # sprite-lab: view sprite animations
#   ./run.sh verify ./scratch  # headless: scripted scenarios + PNG dumps
#   ./run.sh info | gfxtest | shot
#
# The kitty backend needs a terminal speaking the Kitty graphics + keyboard
# protocols (Kitty, Ghostty, foot); the text backends run anywhere. Not tmux.
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

tmux_warn() {
    [[ -n "${TMUX:-}" ]] && echo "warning: running under tmux — Kitty graphics won't pass through." >&2
    return 0
}

# The game with the dev --debug log flag (unless SCAMP_NODEBUG=1).
run_game() {
    local extra=()
    [[ "${SCAMP_NODEBUG:-0}" != "1" ]] && extra+=(--debug)
    tmux_warn
    "target/$profile_dir/scamp" "${extra[@]}"
}

interactive_menu() {
    cargo build "${profile_args[@]}"
    while true; do
        printf '\n  \033[1mSCAMPER\033[0m  \033[2m(munchii)\033[0m\n\n'
        printf '    1  play the game\n'
        printf '    2  sprite viewer  (Tab cycles backends)\n'
        printf '    3  headless verify  (PNG dumps -> ./scratch)\n'
        printf '    4  graphics probe  (gfxtest)\n'
        printf '    5  screenshot to text  (shot)\n'
        printf '    q  quit\n\n'
        read -rp '  > ' choice
        case "$choice" in
            1 | g | game | "") run_game ;;
            2 | s | sprites) "target/$profile_dir/sprite-lab" ;;
            3 | v | verify) "target/$profile_dir/scamp" verify ./scratch ;;
            4 | x | gfxtest) "target/$profile_dir/scamp" gfxtest ;;
            5 | shot) "target/$profile_dir/scamp" shot ;;
            q | Q | quit) break ;;
            *) echo "  ? unknown choice: $choice" ;;
        esac
    done
}

# -i / --interactive: the menu. Otherwise dispatch directly (back-compatible).
case "${1:-}" in
    -i | --interactive | -interactive)
        interactive_menu
        ;;
    sprites | lab | sprite-lab)
        cargo build "${profile_args[@]}" --bin sprite-lab
        exec "target/$profile_dir/sprite-lab"
        ;;
    "")
        cargo build "${profile_args[@]}" --bin scamp
        run_game
        ;;
    *)
        cargo build "${profile_args[@]}" --bin scamp
        tmux_warn
        extra=()
        if [[ "${SCAMP_NODEBUG:-0}" != "1" ]] && [[ ! " $* " == *" --debug "* ]]; then
            extra+=(--debug)
        fi
        exec "target/$profile_dir/scamp" "$@" "${extra[@]}"
        ;;
esac
