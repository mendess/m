#!/bin/bash
if [ -e /tmp ]; then
    TMPDIR=/tmp
else
    TMPDIR="$HOME"
fi

#shellcheck disable=SC2009
last="$(ps -ef |
    grep -v grep |
    grep -oP '\.mpvsocket[0-9]+' |
    sed -E 's/\.mpvsocket([0-9]+)/\1/' |
    sort -V |
    uniq |
    tail -1)"
if [ -z "$last" ]; then
    if [ $# -gt 0 ] && [ "$1" = new ]; then
        echo "$TMPDIR/.mpvsocket0"
    else
        echo "/dev/null"
    fi
else
    if [ $# -gt 0 ] && [ "$1" = new ]; then
        echo "$TMPDIR/.mpvsocket$((++last))"
    else
        echo "$TMPDIR/.mpvsocket$last"
    fi
fi
