#!/usr/bin/env bash
# Build and launch Scamper.
#   ./run.sh                   # play the game (default)
#   ./run.sh -i                # interactive menu (game / sprite viewer / tools)
#   ./run.sh sprites           # sprite-lab: view sprite animations
#   ./run.sh verify ./scratch  # headless: scripted scenarios + PNG dumps
#   ./run.sh record <name>     # play + capture per-tick inputs (q saves)
#   ./run.sh replay <name>     # replay a capture (add --check / --bless headless)
#   ./run.sh info | gfxtest | shot | captures
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
            # NB: assignment form `sel=$(( ))`, not `(( ))` — the latter returns
            # exit status 1 when the result is 0 (moving to the top item), which
            # under `set -e` would kill the whole script.
            $'\e[A' | k) sel=$(( (sel - 1 + n) % n )) ;;
            $'\e[B' | j) sel=$(( (sel + 1) % n )) ;;
            '' | $'\n') MENU_SEL=$sel; printf '\033[?25h'; return ;;
            q | Q | $'\e') MENU_SEL=-1; printf '\033[?25h'; return ;;
        esac
    done
}

# Where captures live — mirrors capture::captures_dir() in the engine.
captures_dir() {
    printf '%s/scamper/captures' "${XDG_STATE_HOME:-$HOME/.local/state}"
}

# A capture name the engine will accept: any single filename component (spaces
# welcome) — just no path separators and not '.'/'..'.
valid_name() {
    [[ -n "$1" && "$1" != "." && "$1" != ".." && "$1" != */* ]]
}

# Read a line into REPLY_NAME with surrounding whitespace trimmed.
read_name() {
    local n
    IFS= read -r n
    # strip leading then trailing whitespace
    n="${n#"${n%%[![:space:]]*}"}"
    n="${n%"${n##*[![:space:]]}"}"
    REPLY_NAME="$n"
}

# Prompt for a name (cursor shown), then record a run. `q` in-game finalizes it.
record_run() {
    local dir name c
    dir="$(captures_dir)"
    printf '\033[2J\033[H\n  \033[1mRecord a run\033[0m\n\n'
    printf '  Name it (spaces are fine); blank cancels.\n  Then play — \033[1mq\033[0m stops and saves the capture.\n\n  name > '
    printf '\033[?25h'
    read_name; name="$REPLY_NAME"
    printf '\033[?25l'
    [[ -z "$name" ]] && return
    if ! valid_name "$name"; then
        printf '\n  invalid name (no "/", not "." or ".."). press a key…'; IFS= read -rsn1; return
    fi
    if [[ -e "$dir/$name.scap" ]]; then
        printf '\n  "%s" exists — overwrite? [y/N] ' "$name"
        IFS= read -rsn1 c; [[ "$c" == y || "$c" == Y ]] || return
    fi
    "target/$profile_dir/scamp" record "$name"
}

# Per-capture action menu: play / rename / delete. (Snapshot golden check/bless
# is a regression-testing tool, kept off the menu — use the flags directly:
#   scamp replay <name> --check    scamp replay <name> --bless)
capture_actions() {
    local dir="$1" name="$2" new c
    while true; do
        menu "capture: $name" \
            "play  (visual replay)" \
            "rename" \
            "delete" \
            "back"
        case "$MENU_SEL" in
            0) "target/$profile_dir/scamp" replay "$name" ;;
            1) printf '\033[2J\033[H\n  rename "%s" to (blank cancels):\n  > ' "$name"
               printf '\033[?25h'; read_name; new="$REPLY_NAME"; printf '\033[?25l'
               if [[ -n "$new" ]] && valid_name "$new"; then
                   mv -f "$dir/$name.scap" "$dir/$new.scap"
                   [[ -e "$dir/$name.snap" ]] && mv -f "$dir/$name.snap" "$dir/$new.snap"
                   return
               fi ;;
            2) printf '\033[2J\033[H\n  delete "%s" (and its golden)? [y/N] ' "$name"
               IFS= read -rsn1 c
               if [[ "$c" == y || "$c" == Y ]]; then
                   rm -f "$dir/$name.scap" "$dir/$name.snap"
                   return
               fi ;;
            *) return ;;
        esac
    done
}

# Browse the captures directory and act on a chosen capture.
replay_browser() {
    local dir f names
    dir="$(captures_dir)"
    while true; do
        names=()
        if [[ -d "$dir" ]]; then
            for f in "$dir"/*.scap; do
                [[ -e "$f" ]] || continue
                names+=("$(basename "$f" .scap)")
            done
        fi
        if [[ ${#names[@]} -eq 0 ]]; then
            printf '\033[2J\033[H\n  \033[1mReplay\033[0m\n\n  no captures yet — record one first.\n\n  press a key…'
            IFS= read -rsn1
            return
        fi
        menu "SCAMPER \xc2\xb7 replay  (pick a capture)" "${names[@]}" "back"
        if [[ $MENU_SEL -lt 0 || $MENU_SEL -ge ${#names[@]} ]]; then
            return
        fi
        capture_actions "$dir" "${names[$MENU_SEL]}"
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
            "record a run" \
            "replay a run" \
            "sprite viewer  (Tab cycles backends)" \
            "tile viewer  (Tab gfx \xc2\xb7 space tile \xc2\xb7 t theme)" \
            "debug tools" \
            "quit"
        case "$MENU_SEL" in
            0) run_game ;;
            1) record_run ;;
            2) replay_browser ;;
            3) "target/$profile_dir/sprite-lab" ;;
            4) "target/$profile_dir/tile-lab" ;;
            5) debug_menu ;;
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
