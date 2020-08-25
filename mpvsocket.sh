#!/bin/bash
if [ -z "$TMPDIR" ]; then
    if [ -e /tmp ]; then
        TMPDIR=/tmp
    else
        TMPDIR="$HOME/.cache"
    fi
fi

CACHE_SOCKET="$HOME/.cache/mpvsocket_cache"

last() {
    case "$1" in
        num) r='[0-9]+' ;;
        *) r='[0-9]+|_cache' ;;
    esac
    #shellcheck disable=SC2009
    ps -ef |
        grep -v grep |
        grep -oP 'mpvsocket('"$r"')' |
        sed -E 's/mpvsocket('"$r"')/\1/' |
        sort -V |
        uniq |
        tail -1
}

case "$1" in
    new)
        last="$(last num)"
        if [ "$last" ]; then
            echo "$TMPDIR/.mpvsocket$((++last))"
        else
            echo "$TMPDIR/.mpvsocket0"
        fi
        ;;
    cache) echo "$CACHE_SOCKET" ;;
    '')
        last="$(last)"
        case "$last" in
            _cache) echo "$CACHE_SOCKET" ;;
            '') echo /dev/null ;;
            *) echo "$TMPDIR/.mpvsocket$last" ;;
        esac
        ;;
esac
