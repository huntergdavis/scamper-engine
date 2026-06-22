#!/usr/bin/env bash
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

# Arrow-key TUI menu. Args: title, then item labels. Sets MENU_SEL to the chosen
# index, or -1 on quit/back (q / Esc).
menu() {
    local title="$1"
    shift
    local items=("$@")
    local n=${#items[@]} sel=0 key rest i
    printf '\033[?25l' # hide cursor
    while true; do
        printf '\033[2J\033[H\n  \033[1m%s\033[0m\n\n' "$title"
        for i in "${!items[@]}"; do
            if [[ $i -eq $sel ]]; then
                printf '   \033[7m  %s  \033[0m\n' "${items[$i]}"
            else
                printf '     %s\n' "${items[$i]}"
            fi
        done
        printf '\n  \033[2m\xe2\x86\x91/\xe2\x86\x93 move \xc2\xb7 enter select \xc2\xb7 q back\033[0m\n'
        IFS= read -rsn1 key
        if [[ $key == $'\e' ]]; then
            # wait a touch for the rest of an arrow's CSI (bare Esc has none)
            IFS= read -rsn2 -t 0.1 rest 2>/dev/null || rest=""
            key+="$rest"
        fi
        case "$key" in
            $'\e[A' | k) ((sel = (sel - 1 + n) % n)) ;;
            $'\e[B' | j) ((sel = (sel + 1) % n)) ;;
            '' | $'\n') MENU_SEL=$sel; printf '\033[?25h'; return ;;
            q | Q | $'\e') MENU_SEL=-1; printf '\033[?25h'; return ;;
        esac
    done
}

debug_menu() {
    while true; do
        menu "SCAMPER \xc2\xb7 debug tools" \
            "headless verify  (PNG dumps -> ./scratch)" \
            "graphics probe  (gfxtest)" \
            "screenshot to text  (shot)" \
            "back"
        case "$MENU_SEL" in
            0) "target/$profile_dir/scamp" verify ./scratch ;;
            1) "target/$profile_dir/scamp" gfxtest ;;
            2) printf '\033[2J\033[H'; "target/$profile_dir/scamp" shot; read -rsn1 -p $'\npress a key…' ;;
            *) return ;;
        esac
    done
}

interactive_menu() {
    cargo build "${profile_args[@]}"
    while true; do
        menu "SCAMPER  (munchii)" \
            "play the game" \
            "sprite viewer  (Tab cycles backends)" \
            "debug tools" \
            "quit"
        case "$MENU_SEL" in
            0) run_game ;;
            1) "target/$profile_dir/sprite-lab" ;;
            2) debug_menu ;;
            *) break ;;
        esac
    done
    printf '\033[2J\033[H'
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
