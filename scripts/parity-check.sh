#!/bin/sh
# parity-check.sh — the stop condition for the Raycast-parity loop.
#
# Every item in docs/PARITY.md that can be decided by a machine is decided
# here, through the internal doors (`_dump`, `_dump-root`, `_dump-all`,
# `_dynamic`, `_footer`, `_actions`) rather than by driving fzf. That is the
# same reason those doors exist.
#
# STATUS is what the scorecard currently claims about an item:
#
#   done   implemented. A failure is a regression and exits non-zero.
#   todo   not implemented. A failure is expected and reported as TODO.
#          A *pass* is reported as READY — promote it to done in the same
#          commit that made it pass, or the check stops protecting anything.
#   spike  blocked on something outside this repository. Never run, never
#          gates. See the item's note in docs/PARITY.md for what would
#          unblock it.
#
# Usage:
#   scripts/parity-check.sh              every item
#   scripts/parity-check.sh P2 P7        named items only
#   PRELUDE=/path/to/prelude scripts/parity-check.sh
#
# Exit: 0 when no `done` item failed, 1 otherwise.

set -u

PRELUDE=${PRELUDE:-./target/release/prelude}
NONSENSE=zzqqxxnonexistentqq

if [ ! -x "$PRELUDE" ]; then
    echo "parity-check: no binary at $PRELUDE (cargo build --release)" >&2
    exit 1
fi

TMP=$(mktemp -d) || exit 1
trap 'rm -rf "$TMP"' EXIT INT TERM

# One gather feeds every item. `_dump-all` is the complete catalogue behind
# scopes, so a per-kind probe is a grep rather than another launch.
"$PRELUDE" _dump-all >"$TMP/all" 2>/dev/null
"$PRELUDE" _dump-root >"$TMP/root" 2>/dev/null

# A rendered row is `<display>\037<json>`. payload() takes the json half.
payload() { awk -F'\037' 'NF>1{print $2}'; }
strip_ansi() { sed 's/\x1b\[[0-9;]*m//g'; }

# row KIND — the first catalogue row of that kind, whole, both halves.
row() { grep -F "\"kind\":\"$1\"" "$TMP/all" | head -1; }

# action_ids ROW — the action panel's verbs, one per line.
action_ids() {
    "$PRELUDE" _actions "$1" 2>/dev/null | strip_ansi |
        awk -F'\037' '{print $1}' | grep -oE '^[a-z][a-z-]*'
}

PASS=0 FAIL=0 TODO=0 READY=0 SKIP=0
WANT=$*

ok()      { PASS=$((PASS + 1)); printf '  \033[32mPASS\033[0m  %s\n' "$1"; }
bad()     { FAIL=$((FAIL + 1)); printf '  \033[31mFAIL\033[0m  %s\n' "$1"; }
todo()    { TODO=$((TODO + 1)); printf '  \033[2mTODO\033[0m  %s\n' "$1"; }
ready()   { READY=$((READY + 1)); printf '  \033[33mREADY\033[0m %s — promote to done\n' "$1"; }
skipped() { SKIP=$((SKIP + 1)); printf '  \033[2mSPIKE\033[0m %s\n' "$1"; }

# item ID STATUS DESCRIPTION — runs check_<ID> and scores it against STATUS.
item() {
    id=$1 status=$2 desc=$3
    case " $WANT " in
        "  ") ;;
        *" $id "*) ;;
        *) return 0 ;;
    esac
    printf '\033[1m%s\033[0m  %s\n' "$id" "$desc"
    if [ "$status" = spike ]; then
        skipped "$id"
        return 0
    fi
    if "check_$id" >"$TMP/why" 2>&1; then
        if [ "$status" = done ]; then ok "$id"; else ready "$id"; fi
    else
        if [ "$status" = done ]; then
            bad "$id"
            sed 's/^/        /' "$TMP/why"
        else
            todo "$id"
            sed 's/^/        /' "$TMP/why"
        fi
    fi
}

# ---------------------------------------------------------------- P1  fallback

# The three properties that make a fallback row safe to emit on every query.
# CLAUDE.md argues each one; they are pinned here because P1b edits this code.
check_P1a() {
    rows=$("$PRELUDE" _dynamic "$NONSENSE" 2>/dev/null | payload)
    n=$(printf '%s\n' "$rows" | grep -c . )
    [ "$n" -ge 1 ] || { echo "no fallback row for an unmatchable query"; return 1; }

    # Display text is the query, or fzf filters the row out with the very
    # query that computed it.
    bad_title=$(printf '%s\n' "$rows" | jq -r --arg q "$NONSENSE" \
        'select(.title != $q) | .title' 2>/dev/null)
    [ -z "$bad_title" ] || { echo "fallback title is not the query: $bad_title"; return 1; }

    # Absent inside a scope: a scope is the person saying where to look.
    scoped=$("$PRELUDE" _dynamic "f:$NONSENSE" 2>/dev/null | payload | grep -c . )
    [ "$scoped" -eq 0 ] || { echo "$scoped fallback row(s) leaked into f:"; return 1; }
}

# Raycast lets you order and extend the fallbacks. Prelude has exactly one,
# hard-wired to the `g` quicklink.
#
# Its own fixture, in a throwaway XDG_CONFIG_HOME, for the reason P3's has one:
# reading the person's configured list asks what they happen to have set, and
# an entry naming a quicklink they later deleted is *correctly* skipped, so the
# count and the config would legitimately disagree.
check_P1b() {
    home="$TMP/xdg-fallbacks/config"
    mkdir -p "$home" || return 1
    fb() { XDG_CONFIG_HOME="$home" "$PRELUDE" "$@"; }

    fb settings get fallbacks >/dev/null 2>&1 || { echo "no 'fallbacks' setting"; return 1; }
    fb settings set fallbacks "g,ddg" >/dev/null 2>&1 ||
        { echo "could not configure two providers"; return 1; }

    rows=$(fb _dynamic "$NONSENSE" 2>/dev/null | payload)
    n=$(printf '%s\n' "$rows" | grep -c . )
    [ "$n" -eq 2 ] || { echo "configured 2 providers, emitted $n"; return 1; }

    # Order is the person's, and P1a's properties hold for every row, not only
    # the first: a second provider whose title was not the query would be
    # filtered out by the query that computed it.
    printf '%s\n' "$rows" | jq -r --arg q "$NONSENSE" 'select(.title != $q) | .title' |
        grep -q . && { echo "a provider row is not titled with the query"; return 1; }
    first=$(printf '%s\n' "$rows" | head -1 | jq -r '.fields[0]')
    second=$(printf '%s\n' "$rows" | sed -n 2p | jq -r '.fields[0]')
    [ "$first" != "$second" ] || { echo "both rows name the same provider"; return 1; }
    case "$first" in *Google*) ;; *) echo "order not preserved: $first before $second"; return 1 ;; esac

    # And a broken list still cannot dead-end a query.
    fb settings set fallbacks zznosuchkeyword >/dev/null 2>&1 || return 1
    n=$(fb _dynamic "$NONSENSE" 2>/dev/null | payload | grep -c . )
    [ "$n" -eq 1 ] || { echo "a list naming nothing emitted $n rows, not the built-in one"; return 1; }
    fb settings check 2>&1 | grep -q zznosuchkeyword ||
        { echo "'settings check' is silent about a keyword that resolves to nothing"; return 1; }

    scoped=$(fb _dynamic "f:$NONSENSE" 2>/dev/null | payload | grep -c . )
    [ "$scoped" -eq 0 ] || { echo "$scoped fallback row(s) leaked into f:"; return 1; }
}

# ----------------------------------------------------------------- P2  aliases

# An alias for any row, not just a quicklink. Refusal happens at the moment of
# naming, on the same three grounds `quicklink_conflict` already refuses.
check_P2() {
    "$PRELUDE" alias list >/dev/null 2>&1 || { echo "no 'alias' verb"; return 1; }
    for taken in "f:" "claude" ; do
        why=$("$PRELUDE" alias add "$taken" zzqqnosuchobject 2>&1)
        if [ $? -eq 0 ]; then
            echo "accepted a reserved alias: $taken"
            "$PRELUDE" alias remove "$taken" >/dev/null 2>&1
            return 1
        fi
        # The refusal has to be about the *name*. Exit status alone passed
        # while the target was resolved first and the key never examined —
        # right answer, wrong question, and the check could not tell.
        case "$why" in
            *"$taken"*) ;;
            *) echo "refused $taken without naming it: $why"; return 1 ;;
        esac
    done
}

# The alias belongs on the row that has it, in `fields` — not in a sixth column.
#
# This brings its own fixture, in a throwaway XDG_CONFIG_HOME. It first read
# `_dump-root` for any row carrying an alias, which asked whether the person
# happened to have named something that happens to be a root command — a
# question about their config rather than about the code.
check_P3() {
    home="$TMP/xdg/config"
    mkdir -p "$home" || return 1
    run() { XDG_CONFIG_HOME="$home" "$PRELUDE" "$@"; }

    target=$(grep -F '"kind":"app"' "$TMP/all" | head -1 | payload | jq -r .title)
    [ -n "$target" ] || { echo "no application in the catalogue to name"; return 1; }

    run alias add zzparitycheck "$target" >/dev/null 2>&1 ||
        { echo "could not name “$target”"; return 1; }
    row=$(run _dump-all 2>/dev/null | payload |
        jq -c 'select(.data.alias == "zzparitycheck")' | head -1)
    run alias remove zzparitycheck >/dev/null 2>&1

    [ -n "$row" ] || { echo "no row carries the alias"; return 1; }
    printf '%s' "$row" | jq -e '.fields[0] == "zzparitycheck"' >/dev/null ||
        { echo "the alias is not first in fields: $row"; return 1; }
    # `render_general` shows fields *or* the subtitle. A row whose detail lives
    # in its subtitle must not lose it the moment it acquires a name.
    printf '%s' "$row" |
        jq -e '(.subtitle // "") as $s | $s == "" or ((.fields // []) | index($s)) != null' >/dev/null ||
        { echo "the alias evicted the subtitle: $row"; return 1; }

    # The other half. A collection that can be built from the CLI and never
    # seen in the launcher is the state `ql:` was built to end.
    run settings path aliases >/dev/null 2>&1 || { echo "no 'aliases' setting"; return 1; }
    run _dump-all 2>/dev/null | payload |
        jq -e 'select(.kind == "setting" and .data.setting == "aliases" and .data.edit == "collection")' \
        >/dev/null || { echo "no Aliases collection row in the settings form"; return 1; }
}

# --------------------------------------------------- P4/P5  launch with a query

# P4a was struck by the P5 decision: `prelude://` acts rather than showing a
# launcher, so nothing needs a seeded query as a payload. P4b folded into P5.
# See "Struck" in docs/PARITY.md.

# --------------------------------------------------------------- P5  deeplinks

# Written with the item. It must prove the handler *receives* a URL and not
# merely that the scheme is claimed: a bundle whose executable is a shell
# script claims it correctly and then silently never runs, because the URL
# arrives as an Apple Event rather than as argv.
check_P5() { echo "not written yet"; return 1; }

# P6 was struck. It claimed the action panel names the verb and not the noun,
# which was a misreading: `panel()` states the verb in the header and the row
# on the border label two lines below, and every level under it reuses that
# same title. See "Struck" in docs/PARITY.md.

# ------------------------------------------------------------- P7  pin any row

# Favorites cover Agent, Skill and MCP object keys. An app and a saved
# quicklink have stable keys too and cannot be promoted.
#
# Promotion stays *inside* the Kind band. Do not reach for `by_rank`: a pin
# that crosses bands is the arithmetic CLAUDE.md already took apart, and a
# test walks every pair of kinds to keep it taken apart.
check_P7() {
    missing=
    for k in app link; do
        r=$(row "$k")
        [ -n "$r" ] || continue
        action_ids "$r" | grep -qx 'favorite\|unfavorite' || missing="$missing $k"
    done
    [ -z "$missing" ] || { echo "no favorite action on:$missing"; return 1; }
}

# P8 was struck. It claimed a trashed object does not say where it went and
# offers no way back; all three trash paths already print the destination, and
# `~/.Trash` is already an openable Folder object. See "Struck" in
# docs/PARITY.md.

# ------------------------------------------------- P10  the object chords hold

# `ui::object_of` asks once, and three chords plus their footer labels read
# that one answer. This is the regression guard for everything above it.
check_P10() {
    for k in session skill; do
        r=$(row "$k")
        [ -n "$r" ] || continue
        "$PRELUDE" _footer "$r" 2>/dev/null | strip_ansi | grep -q 'Ctrl+Option+Enter' ||
            { echo "$k row lost its object chords"; return 1; }
    done
    r=$(row agent)
    if [ -n "$r" ]; then
        ! "$PRELUDE" _footer "$r" 2>/dev/null | strip_ansi | grep -q 'Ctrl+Option+Enter' ||
            { echo "agent row grew object chords it has no object for"; return 1; }
    fi
}

# ---------------------------------------------------------------------- runner

printf '\033[1mRaycast parity — interaction polish\033[0m\n\n'

item P1a done  "Fallback row: is the query, survives, absent in a scope"
item P1b done  "Fallbacks are an ordered, configurable list"
item P2  done  "Alias a stable object, refused at the moment of naming"
item P3  done  "The alias shows on the row, and has a manager"
item P5  todo  "prelude:// deeplinks, which act rather than show"
item P7  done  "Pin an app or a quicklink, inside its band"
item P10 done  "Object chords appear on object rows and nowhere else"

printf '\n  %d pass · %d fail · %d todo · %d ready · %d spike\n' \
    "$PASS" "$FAIL" "$TODO" "$READY" "$SKIP"

[ "$READY" -gt 0 ] && printf '  \033[33m%d item(s) pass while marked todo — promote them.\033[0m\n' "$READY"
[ "$FAIL" -gt 0 ] && exit 1
exit 0
