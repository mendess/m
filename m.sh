#!/bin/bash
#shellcheck disable=SC2119
#shellcheck disable=SC2155

#shellcheck disable=SC1090
[ -e ~/.config/user-dirs.dirs ] && . ~/.config/user-dirs.dirs

CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config/}/m"
PLAYLIST="$(realpath "$CONFIG_DIR/playlist")"
SCRIPT_NAME="$(basename "$0")"
if [ -z "$TMPDIR" ]; then
    if [ -e /tmp ]; then
        TMPDIR=/tmp
    else
        TMPDIR="$HOME/.cache"
    fi
fi

readonly MUSIC_DIR="${XDG_MUSIC_DIR:-$HOME/Music}"
mkdir -p "$MUSIC_DIR"
LOOP_PLAYLIST="--loop-playlist"
WITH_VIDEO=no

mkdir -p "$CONFIG_DIR"

case "${1,,}" in
    gui)
        PROMPT_PROG=dmenu
        shift
        ;;
    *)
        PROMPT_PROG=fzf
        ;;
esac

error() {
    notify --error "$@" >&2
}

update_panel() {
    [ ! -e "$CONFIG_DIR/update_panel.sh" ] || sh "$CONFIG_DIR/update_panel.sh"
}

check_cache() {
    local PATTERN
    [[ -z "$1" ]] && error wtf && exit 1
    PATTERN=("$MUSIC_DIR"/*"$(basename "$1" | grep -Eo '.......$')"*)
    if [[ -f "${PATTERN[0]}" ]]; then
        echo "${PATTERN[0]}"
    else
        echo "$1"
        grep -q "$1" "$PLAYLIST" &&
            [[ "$(pgrep -f youtube-dl | wc -l)" -lt 8 ]] &&
            youtube-dl -o "$MUSIC_DIR/"'%(title)s-%(id)s=m.%(ext)s' \
                --add-metadata \
                "$1" &>"$TMPDIR/youtube-dl" &
    fi
}

selector() {
    while [ "$#" -gt 0 ]; do
        case "$1" in
            -l) local listsize="$2" ;;
            -p) local prompt="$2" ;;
        esac
        shift
    done
    case "$PROMPT_PROG" in
        fzf) fzf -i --prompt "$prompt " ;;
        dmenu) dmenu -i -p "$prompt" -l "$listsize" ;;
    esac
}

notify() {
    bold() { if [ -t 1 ] || [ -t 2 ]; then echo -en "\e[1m$1\e[0m"; else echo -en "$1"; fi; }
    red() { if [ -t 1 ] || [ -t 2 ]; then echo -en "\e[1;31m$1\e[0m"; else echo -en "$1"; fi; }
    local text=()
    while [ $# -gt 0 ]; do
        case "$1" in
            -i)
                shift
                local img="$1"
                ;;
            -e | --error)
                local err="1"
                ;;
            *)
                text+=("$1")
                ;;
        esac
        shift
    done
    tty() {
        bold "${text[0]}\n"
        if [ -n "${text[1]}" ]; then echo -e "${text[1]}"; fi # don't change to short form
    }
    if [ "$PROMPT_PROG" = fzf ]; then
        if [[ "$err" ]]; then
            red "Error: " >&2
            tty >&2
        else
            tty
        fi
    else
        local args=("${text[@]}")
        args+=(-a "$SCRIPT_NAME")
        [ -n "$img" ] && args+=(-i "$img")
        [ -n "$err" ] && args+=(--urgency critical)
        notify-send "${args[@]}"
    fi
}

with_video() {
    if [ -z "$DISPLAY" ]; then
        WITH_VIDEO=no
    elif [ "$1" = force ] || [ "$(mpvsocket)" = /dev/null ]; then
        WITH_VIDEO=$(printf "no\nyes" | selector -i -p "With video?")
    fi
}

play() {
    mpv_do '["set_property", "pause", true]' &>/dev/null
    case $WITH_VIDEO in
        yes)
            mpv \
                --geometry=820x466 \
                "$LOOP_PLAYLIST" \
                --input-ipc-server="$(mpvsocket new)" \
                "$@"
            ;;
        no)
            if [ -z "$DISPLAY" ]; then
                mpv \
                    --geometry=820x466 \
                    "$LOOP_PLAYLIST" \
                    --input-ipc-server="$(mpvsocket new)" \
                    --no-video "$@"
            else
                setsid mpv \
                    --geometry=820x466 \
                    "$LOOP_PLAYLIST" \
                    --input-ipc-server="$(mpvsocket new)" \
                    --no-video \
                    "$@" &>/dev/null &
                disown
            fi
            ;;
    esac
}

songs_in_cat() {
    sed '/^$/ d' "$PLAYLIST" |
        grep -P ".*\t.*\t.*\t.*$1" |
        awk -F'\t' '{print $2}'
}

start_playlist_interactive() {
    local modes="All
single
random
Category
clipboard"

    local mode=$(echo "$modes" |
        selector -i -p "Mode?" -l "$(echo "$modes" | wc -l)")

    local vidlist
    vidlist=$(sed '/^$/ d' "$PLAYLIST")

    case "$mode" in
        single)
            local vidname="$(echo "$vidlist" |
                awk -F'\t' '{print $1}' |
                tac |
                selector -i -p "Which video?" -l "$(echo "$vidlist" | wc -l)")"

            if [ -z "$vidname" ]; then
                return 1
            else
                local vids="$(echo "$vidlist" |
                    grep -F "$vidname" |
                    awk -F'\t' '{print $2}')"
            fi
            LOOP_PLAYLIST=""
            ;;

        random)
            local vids="$(echo "$vidlist" |
                shuf |
                sed '1q' |
                awk -F'\t' '{print $2}')"
            LOOP_PLAYLIST=""
            ;;

        All)
            local vids="$(echo "$vidlist" |
                shuf |
                awk -F'\t' '{print $2}' |
                xargs)"
            ;;

        Category)
            local catg=$(echo "$vidlist" |
                awk -F'\t' '{for(i = 4; i <= NF; i++) { print $i } }' |
                tr '\t' '\n' |
                sed '/^$/ d' |
                sort |
                uniq -c |
                selector -i -p "Which category?" -l 30 |
                sed -E 's/^[ ]*[0-9]*[ ]*//')

            [ -z "$catg" ] && return 1
            vidlist=$(echo "$vidlist" | shuf)
            local vids="$(songs_in_cat "$catg" | shuf | xargs)"
            ;;

        clipboard)
            local clipboard=1
            local vids="$(xclip -sel clip -o)"
            [ -n "$vids" ] || return 1
            LOOP_PLAYLIST=""
            ;;
        *)
            return 1
            ;;
    esac

    [ -z "$vids" ] && return 1

    with_video

    local final_list=()
    for v in $(echo "$vids" | shuf); do
        final_list+=("$(check_cache "$v")")
    done

    [ -z "$clipboard" ] &&
        (
            cd "$MUSIC_DIR" || return 1
            printf "%s\n" "${final_list[@]}" |
                grep '^http' |
                xargs \
                    --no-run-if-empty \
                    -L 1 \
                    youtube-dl -o '%(title)s-%(id)s=m.%(ext)s' \
                    --add-metadata &>"$TMPDIR/youtube-dl"
        ) &

    if [ "$(mpvsocket)" != "/dev/null" ]; then
        for song in "${final_list[@]}"; do
            [[ "$song" == *playlist* ]] &&
                local playlist=1 &&
                break
        done
        if [ "$playlist" ]; then
            for song in "${final_list[@]}"; do
                if [[ "$song" == *playlist* ]]; then
                    for s in $(youtube-dl "$song" --get-id); do
                        main queue "https://youtu.be/$s" --notify
                    done
                else
                    main queue "$song" --notify
                fi
            done
        else
            local cmd=(queue "${final_list[@]}" --notify)
            [ "$mode" = All ] && cmd+=(--no-move)
            "${cmd[@]}"
        fi
    else
        (
            sleep 2
            update_panel
            sleep 5
            m queue "${final_list[@]:10}" --no-move --no-preempt-download
        ) &
        local starting_queue=("${final_list[@]:0:10}")
        play "${starting_queue[@]}"
    fi
}

mpv_do() {
    echo '{ "command": '"$1"' }' | socat - "$(mpvsocket)" |
        if [ "$2" ]; then jq "${@:2}"; else cat; fi
}

mpv_get() {
    mpv_do '["get_property", "'"$1"'"]' "${@:2}"
}

spotify_toggle_pause() {
    dbus-send --print-reply \
        --dest=org.mpris.MediaPlayer2.spotify \
        /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player.PlayPause
}

spotify_next() {
    dbus-send --print-reply \
        --dest=org.mpris.MediaPlayer2.spotify \
        /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player.Next
}

spotify_prev() {
    dbus-send --print-reply \
        --dest=org.mpris.MediaPlayer2.spotify \
        /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player.Previous
}

mpvsocket() {
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
}

up_next() {
    next_id="$(($(mpv_get 'playlist-pos' .data) + 1))"

    videoId="$(mpv_get playlist .data["$next_id"].filename -r)"

    if [[ "$videoId" = *youtu* ]]; then
        id="$(echo "$videoId" | sed -r 's|.*/([^/]+)/?$|\1|g')"
        filename=$(awk -F '\t' '$2 ~ /'"$id"'/ {print $1}' "$PLAYLIST")
        [ -z "$filename" ] && filename=$(youtube-dl --get-title "$videoId")
    else
        filename=$(basename "$videoId" |
            sed -r 's/\.[^.]+$//' |
            sed -r 's/-[a-zA-Z\-_0-9]{11}$//')
    fi
    width=40
    [ "${#filename}" -gt $width ] && width="${#filename}"
    [ -n "$filename" ] && [ "$filename" != null ] &&
        echo "=== UP NEXT ===" &&
        echo "$filename"
}

current_song() {
    local filename videoId chapter categories up_next
    videoId="$(mpv_get filename --raw-output '.data' |
        sed -E 's/.*-([a-zA-Z0-9\-_-]{11})(=m)?.*/\1/g')"
    [[ "$1" =~ (-i|--link) ]] && notify "https://youtu.be/$videoId" && return

    filename=$(mpv_get media-title --raw-output '.data')

    chapter=$(mpv_get chapter-metadata '.data.title' -r)

    if [ -z "$filename" ] ||
        [ "$filename" = "_" ] ||
        [ "$filename" = "$videoId" ]; then

        [ -z "$videoId" ] && return 1
        filename=$(grep "$videoId" "$PLAYLIST" | awk -F '\t' '{print $1}')
        [ -z "$filename" ] && filename="$videoId"
    fi
    local status
    case "$(mpv_get pause --raw-output .data)" in
        true) status="||" ;;
        false) status=">" ;;
    esac
    local volume="$(mpv_get volume --raw-output .data)"
    [[ "$1" =~ (-s|--short) ]] && {
        if [ -n "$chapter" ] && [ "$chapter" != "null" ]; then
            notify "Video: $filename Song: $chapter $status ${volume}%"
        else
            notify "$filename $status ${volume}%"
        fi
        return
    }
    width=40
    [ "${#filename}" -gt $width ] && width="${#filename}"
    categories=$(awk -F'\t' '/'"$videoId"'/ {
            for(i = 4; i <= NF; i++) {
                acc = acc " | " $i
            };
            print("Categories:"acc" |")
        }' "$PLAYLIST" |
        fold -s -w "$width")
    [[ ! "$1" =~ (-n|--notify) ]] && filename="$filename
$status 🔉${volume}%"

    if [ "$categories" != 'Categories: |' ]; then
        filename="$filename
$categories"
    fi
    up_next="$(up_next)"
    [ -n "$up_next" ] && filename="$filename

$up_next"
    local pprog="$PROMPT_PROG"
    [[ "$1" =~ (-n|--notify) ]] && PROMPT_PROG=dmenu
    notify "Now Playing" "$filename"
    PROMPT_PROG="$pprog"
}

add_cat() {
    local cat
    local current_song=$(PROMPT_PROG=fzf current_song --link |
        tail -1 |
        sed 's/"//g' |
        sed -E 's|.*/([^/]+)$|\1|g')

    [ -z "$current_song" ] && return 2

    while :; do
        case "$PROMPT_PROG" in
            dmenu)
                cat=$(echo | dmenu -p "Category name? (Esq to quit)")
                ;;
            fzf)
                read -r -p "Category name [Empty to quit]? " cat || echo
                ;;
        esac
        if [ -z "$cat" ]; then
            break
        fi
        sed -i -E "/$current_song/ s|$|	$cat|" "$PLAYLIST"
    done
}

last_queue() {
    case "$1" in
        reset)
            rm -f "$(last_queue)"
            ;;
        *)
            echo "$(mpvsocket)_last_queue"
            ;;
    esac
}

interpret_song() {
    local targets search search_terms=()
    INTERPRET_targets=()
    INTERPRET_reseted=
    INTERPRET_no_move=
    INTERPRET_no_preempt_download=
    INTERPRET_notify=
    INTERPRET_clear=
    while [ $# -gt 0 ]; do
        case "$1" in
            -r | --reset)
                if [[ -z "$INTERPRET_QUEUE_OPTIONS" ]]; then
                    error "Queue only option: $1"
                    return 1
                fi
                INTERPRET_reseted=1
                ;;
            -m | --no-move)
                if [[ -z "$INTERPRET_QUEUE_OPTIONS" ]]; then
                    error "Queue only option: $1"
                    return 1
                fi
                INTERPRET_no_move=1
                ;;
            -d | --no-preempt-download)
                if [[ -z "$INTERPRET_QUEUE_OPTIONS" ]]; then
                    error "Queue only option: $1"
                    return 1
                fi
                INTERPRET_no_preempt_download=1
                ;;
            -n | --notify)
                if [[ -z "$INTERPRET_QUEUE_OPTIONS" ]]; then
                    error "Queue only option: $1"
                    return 1
                fi
                INTERPRET_notify=1
                ;;
            -x | --clear)
                if [[ -z "$INTERPRET_QUEUE_OPTIONS" ]]; then
                    error "Queue only option: $1"
                    return 1
                fi
                INTERPRET_clear=1
                ;;
            -s | --search)
                search=1
                ;;
            --search=*)
                search=1
                search_terms+=("${1#*=}")
                ;;
            -c | --category)
                shift
                while read -r line; do
                    targets+=("$(check_cache "$line")")
                done < <(songs_in_cat "$1")
                ;;
            --category=*)
                while read -r line; do
                    targets+=("$(check_cache "$line")")
                done < <(songs_in_cat "${1#*=}")
                ;;
            http*)
                targets+=("$(check_cache "$1")")
                ;;
            -?*)
                error "Invalid option:" "$1"
                return 1
                ;;
            *)
                if [[ -e "$1" ]]; then
                    targets+=("$1")
                else
                    search_terms+=("$1")
                fi
                ;;
        esac
        shift
    done
    if [[ "$search" ]]; then
        targets+=("ytdl://ytsearch:${search_terms[*]}")
    elif [[ "${#search_terms[@]}" -gt 0 ]]; then
        local t
        for term in "${search_terms[@]}"; do
            t="$(if [[ "$t" ]]; then echo "$t"; else cat "$PLAYLIST"; fi |
                awk \
                    -v IGNORECASE=1 \
                    -F '\t' \
                    '$1 ~ /'"$term"'/ {print $1"\t"$2}' |
                while IFS=$'\t' read -r name link _; do
                    printf "%s\t%s\n" "$name" "$link"
                done)"
        done
        if [[ -z "$t" ]]; then
            [[ "${#targets[@]}" = 0 ]] &&
                error "No matches" &&
                return 1
        else
            [[ "$(echo "$t" | wc -l)" -gt 1 ]] &&
                error "Too many matches" &&
                return 1
            targets+=("$(check_cache "$(echo "$t" | cut -f2)")")
        fi
    fi
    [[ "${#targets[@]}" -gt 0 ]] &&
        mapfile -t INTERPRET_targets < <(printf "%s\n" "${targets[@]}" | sort -u | shuf)
    return 0
}

queue() {
    INTERPRET_QUEUE_OPTIONS=1 interpret_song "$@" || return 1
    [[ "${#INTERPRET_targets[@]}" -lt 1 ]] &&
        [[ ! "$INTERPRET_reseted" ]] &&
        error "No files to queue" &&
        return 1
    [[ "$INTERPRET_clear" ]] &&
        echo -n "Clearing playlist... " &&
        mpv_do '["playlist-clear"]' --raw-output .error
    [[ "$INTERPRET_clear$INTERPRET_reseted" ]] &&
        notify "Reseting queue..." &&
        last_queue reset

    if [[ "$(mpvsocket)" = /dev/null ]]; then
        with_video force
        play "${INTERPRET_targets[@]}"
        return
    fi
    for file in "${INTERPRET_targets[@]}"; do
        echo -n "Queueing song: '$file'... "
        mpv_do '["loadfile", "'"$file"'", "append"]' --raw-output .error
        if [[ "$INTERPRET_no_move" ]]; then
            local playlist_pos=$(mpv_get playlist-count --raw-output '.data')
        else
            local count current target last_queue
            count=$(mpv_get playlist-count --raw-output '.data')
            current=$(mpv_get playlist-pos --raw-output '.data')

            target=$((current + 1))
            last_queue="$(last_queue)"
            [ -e "$last_queue" ] &&
                [ "$target" -le "$(cat "$last_queue")" ] &&
                target=$(($(cat "$last_queue") + 1))
            echo -n "Moving from $count -> $target ... "
            mpv_do '["playlist-move", '$((count - 1))', '$target']' --raw-output .error
            echo "$target" >"$last_queue"
            local playlist_pos=$target
        fi
        [ "$INTERPRET_notify" = 1 ] && {
            local img img_back name
            img=$(mktemp --tmpdir tmp.XXXXXXXXXXXXXXXXX.png)
            img_back="${img}_back.png"
            if [[ "$file" == https* ]]; then
                local data
                data=$(youtube-dl --get-title "$file" --get-thumbnail)
                name=$(echo "$data" | head -1)
                echo "$data" | tail -1 | xargs -r wget --quiet -O "$img"
                [ -z "$name" ] && name="$file"
            else
                name="$(ffprobe "$file" 2>&1 |
                    grep title |
                    cut -d':' -f2 |
                    xargs)"
                ffmpeg \
                    -y \
                    -loglevel error \
                    -hide_banner \
                    -vsync 2 \
                    -i "$file" \
                    -frames:v 1 \
                    "$img" >/dev/null
            fi
            convert -scale x64 -- "$img" "$img_back" && mv "$img_back" "$img"
            PROMPT_PROG=dmenu notify "Queued '$name'" \
                "$([ "$current" ] &&
                    printf "Current: %s\nQueue pos: %s" "$current" "$target")" \
                -i "$img"
            rm -f "$img"
        } &
        [[ -z "$INTERPRET_no_preempt_download" ]] && [[ "$file" =~ (ytdl|http).* ]] &&
            case "$file" in
                *playlist*) echo "preempt download is not available for playlists" ;;
                *) preempt_download "$playlist_pos" "$file" ;;
            esac &
        disown
        if [ "$(jobs -p | wc -l)" -ge "$(nproc)" ]; then
            wait -n
        fi
    done
    wait
    [ ${#INTERPRET_targets[@]} -ge 5 ] && last_queue reset
    :
}

dequeue() {
    local to_remove
    case "$1" in
        next)
            dequeue +1
            ;;
        prev)
            dequeue -1
            ;;
        +[0-9]*)
            to_remove="$(($(mpv_get playlist-pos -r .data) + ${1#+}))"
            ;;
        -[0-9]*)
            to_remove="$(($(mpv_get playlist-pos -r .data) - ${1#-}))"
            ;;
        [0-9]*)
            to_remove="$1"
            ;;
    esac
    [[ ! "$to_remove" ]] && return
    mpv_do "[\"playlist-remove\", \"$to_remove\"]" .error
}

preempt_download() {
    local queue_pos="$1"
    case "$2" in
        ytdl://ytsearch:*)
            local link="ytsearch1:${2#ytdl://ytsearch:}"
            ;;
        *)
            local link="$2"
            ;;
    esac
    local id="$(youtube-dl "$link" --get-id)" || return

    readonly local cache_dir="${XDG_CACHE_HOME:-$HOME/.cache}/queue_cache"
    mkdir -p "$cache_dir"
    local filename="$cache_dir/$id.m4a"

    [[ ! -e "$filename" ]] &&
        youtube-dl "$link" \
            --format 'bestaudio[ext=m4a]' \
            --add-metadata \
            --output "$cache_dir/"'%(id)s.%(ext)s' \
            &>"$cache_dir/$id.log" || return

    touch "$filename"
    mpv_do '["loadfile", "'"$filename"'", "append"]' >/dev/null
    mpv_do '["playlist-remove", '"$queue_pos"']' >/dev/null
    local count=$(mpv_get playlist-count --raw-output '.data')
    mpv_do '["playlist-move", '$((count - 1))', '"$queue_pos"']' >/dev/null
    find "$cache_dir" -type f -mtime +1 -delete
}

now() {
    local current start end range back_offset
    current="$(mpv_get playlist-pos | jq .data)"
    local range=${1:-10}
    back_offset=$(python -c "import math; print(math.floor($range*0.2) - 1)")
    start="$((current - back_offset))"
    case "$start" in
        -* | 0) start="1" ;;
    esac
    end="$((start + range - 1))"
    #shellcheck disable=SC2016
    mpv_get playlist -r '.data | .[] | .filename' |
        sed -n "${start},${end}p;$((end + 1))q;" |
        perl -ne 's|^.*/([^/]*?)(-[A-Za-z0-9\-_-]{11}=m)?\.[^./]*$|\1\n|; print' |
        sed -r 's/^$/=== ERROR ===/g' |
        python -c 'from threading import Thread
import fileinput
from subprocess import check_output as popen

def get_title(i, x):
    fetch = lambda: popen(["youtube-dl", "--get-title", x]).decode("utf-8").strip()
    try:
        titles[i] = fetch() if x.startswith("http") else x
    except:
        titles[i] = f"Error fetching song title: `{x}`"

i = 0
titles = []
ts = []
for line in fileinput.input():
    titles.append(None)
    t = Thread(target=get_title, args=(i, line.strip()))
    t.start()
    ts.append(t)
    i += 1

for i in range(len(ts)):
    if ts[i]:
        ts[i].join()
        if titles[i]:
            print(titles[i])' |
        awk -v current="$current" -v pos="$((--start))" \
            '{
            if (pos != current) {
                printf("%3d     %s\n", pos, $0)
            } else {
                printf("%3d ==> %s\n", pos, $0);
            }
            pos++
        }'
}

add_song() {
    url="$(echo "$1" | sed -E 's|https://.*=(.*)\&?|https://youtu.be/\1|')"
    [ -z "$url" ] && error "'$url' is not a valid link" && return 1
    entry="$(grep "$url" "$PLAYLIST")" &&
        error "Song already in $PLAYLIST" "$entry" &&
        return 1
    categories=$(echo "${@:2}" |
        tr '[:upper:]' '[:lower:]' |
        tr ' ' '\t' |
        sed -E 's/\t$//')
    [ -n "$categories" ] && categories="	$categories"
    notify 'getting title'
    title="$(youtube-dl --get-title "$1" | sed -e 's/(/{/g; s/)/}/g' -e "s/'//g")"
    [ "${PIPESTATUS[0]}" -ne 0 ] &&
        error 'Failed to get title from output' &&
        return 1

    notify 'getting duration'
    duration="$(youtube-dl --get-duration "$1" |
        sed -E 's/(.*):(.+):(.+)/\1*3600+\2*60+\3/;s/(.+):(.+)/\1*60+\2/' |
        bc -l)"
    [ "${PIPESTATUS[0]}" -ne 0 ] &&
        error 'Failed to get duration from output' &&
        return 1

    notify 'adding to playlist'
    echo "$title	$url	$duration$categories"
    echo "$title	$url	$duration$categories" >>"$PLAYLIST"
}

del_song() {
    local search
    if [[ $# -lt 1 ]]; then
        echo "missing argument"
        return 1
    elif [[ "$1" =~ --current|-c ]]; then
        search="$(current_song --link)"
    else
        search="$*"
    fi
    num_results="$(grep -c -i "$search" "$PLAYLIST")"
    results="$(awk -F'\t' -v IGNORECASE=1 '/'"$*"'/ {print $1}' "$PLAYLIST")"
    case "$num_results" in
        0) error 'no results' && return 1 ;;
        1)
            notify 'Deleting song' "$results"
            sed -i '/'"$*"'/Id' "$PLAYLIST"
            ;;
        *) error 'too many results' "$results" && return 1 ;;
    esac
}

clean_dl_songs() {
    find "$MUSIC_DIR"/ -maxdepth 1 -mindepth 1 -type f |
        grep -E -e '-[A-Za-z0-9\-_-]{11}=m\.[^.]{3,4}$' |
        sed -E 's/^.*-([A-Za-z0-9\-_-]{11})=m.*$/\1/g' |
        (
            while read -r id; do
                grep -F -e "$id" "$PLAYLIST" &>/dev/null && continue
                PATTERN=("$MUSIC_DIR"/*"$id"*)
                [ -e "${PATTERN[0]}" ] && {
                    [ -z "$b" ] && echo "cleaning downloads" && b='done'
                    rm -v "${PATTERN[0]}"
                }
            done
            [ "$b" ] && echo "Done"
        )
}

loop() {
    looping="$(mpv_get loop-playlist | jq -r .data)"
    case "$looping" in
        inf)
            arg=no
            msg=not
            ;;
        false)
            arg=inf
            msg=now
            ;;
    esac
    e="$(mpv_do "[\"set_property\", \"loop-playlist\", \"$arg\"]" |
        jq -r .error)"
    case "$e" in
        success)
            notify "$msg looping"
            ;;
        *) error "$e" ;;
    esac
}

main() {
    case $1 in
        p | pause)
            ## Toggle pause
            if pgrep spotify &>/dev/null; then
                spotify_toggle_pause
            else
                echo 'cycle pause' | socat - "$(mpvsocket)"
                update_panel &
                disown
            fi
            ;;
        quit)
            ## Kill the most recent player
            echo 'quit' | socat - "$(mpvsocket)"
            update_panel &
            disown
            ;;
        play)
            ## Play something
            ##      Usage: m play [options] link
            ## Options:
            ##      -s | --search  Search the song on youtube
            interpret_song "${@:2}" || exit 1
            with_video force
            [[ "${#INTERPRET_targets[@]}" -eq 1 ]] && LOOP_PLAYLIST=""
            play "${INTERPRET_targets[@]}"
            ;;
        playlist | play-interactive)
            ## Interactively asks the user what songs they want to play
            ## from their playlist
            [ -e "$PLAYLIST" ] || touch "$PLAYLIST"
            [ ! -s "$PLAYLIST" ] && error "Playlist file emtpy" && return 1
            start_playlist_interactive
            ;;
        new | add-song)
            ## Add a new song
            ##      Usage: m add-song [options] link [category,..]
            ## Options:
            ##      -q | --queue  Queue the song too
            case $2 in
                -q | --queue) m queue "$3" ;;
            esac
            add_song "${@:2}"
            ;;
        add-playlist)
            ## Append a playlist to the personal playlist
            ##      Usage: m add-playlist [options] [link] [category,..]
            ## Options:
            ##      -q | --queue  Queue the playlist too
            case $2 in
                -q | --queue)
                    m queue "$3"
                    shift
                    ;;
            esac
            youtube-dl --get-id "$2" |
                sed 's|^|https://youtu.be/|' |
                while read -r l; do
                    notify "adding $l"
                    main add-song "$l" "${@:3}"
                done
            ;;
        cat)
            ## List all current categories
            cut -f4- "$PLAYLIST" | tr '\t' '\n' | grep -vP '^$' | sort | uniq -c | sort -n
            ;;
        now)
            ## Shows the current playlist
            now "${@:2}"
            ;;
        c | current)
            ## Show the current song
            ## Options:
            ##      -n | --notify  With a notification
            ##      -i | --link    Print the filename / link instead
            current_song "${@:2}"
            ;;
        add-cat-to-current | new-cat)
            ## Add a category to the current song
            add_cat "${@:2}"
            ;;
        q | queue)
            ## Queue a song
            ## Options:
            ##     -r | --reset               Resets the queue fairness
            ##     -s | --search STRING       Searches youtube for the STRING
            ##     -n | --notify              Send a notification
            ##     -m | --no-move             Don't move in the playlist, keep it at the end
            ##     -c | --category STRING     Queue all songs in a category
            ##     -p | --no-preempt-download Don't preemptively download songs
            queue "${@:2}"
            ;;
        dq | dequeue)
            dequeue "${@:2}"
            ;;
        del | delete-song)
            ## Delete a passed song
            ## Options:
            ##      -c | --current Delete the current song
            del_song "${@:2}"
            ;;
        clean-downloads)
            ## Clears downloads that are no longer in the playlist
            clean_dl_songs
            ;;
        loop)
            ## Toggles playist looping
            loop
            ;;
        k | vu)
            ## Increase volume
            ## Usage: m vu [amount]
            ##  default amount is 2
            echo "add volume ${2:-2}" | socat - "$(mpvsocket)"
            update_panel &
            disown
            ;;
        j | vd)
            ## Decrease  volume
            ## Usage: m vd [amount]
            ##  default amount is 2
            echo "add volume -${2:-2}" | socat - "$(mpvsocket)"
            update_panel &
            disown
            ;;
        H | prev)
            ## Previous chapter in a file
            echo 'add chapter -1' | socat - "$(mpvsocket)"
            update_panel &
            disown
            {
                sleep 2
                update_panel
            } &
            ;;
        L | next)
            ## Next chapter in a file
            echo 'add chapter 1' | socat - "$(mpvsocket)"
            update_panel &
            disown
            ;;
        h | prev-file)
            ## Go to previous file
            if pgrep spotify &>/dev/null; then
                spotify_prev
                update_panel &
                disown
            else
                echo 'playlist-prev' | socat - "$(mpvsocket)"
            fi
            ;;
        l | next-file)
            ## Skip to the next file
            if pgrep spotify &>/dev/null; then
                spotify_next
                update_panel &
                disown
            else
                echo 'playlist-next' | socat - "$(mpvsocket)"
            fi
            ;;
        J | u | back)
            ## Seek backward
            ## Usage: m back [amount]
            ##  default amount is 2 seconds
            echo "seek -${2:-10}" | socat - "$(mpvsocket)"
            ;;
        K | i | frwd)
            ## Seek forward
            ## Usage: m frwd [amount]
            ##  default amount is 2 seconds
            echo "seek ${2:-10}" | socat - "$(mpvsocket)"
            ;;
        int | interactive)
            ## Enter interactive mode
            main r
            while :; do
                read -r -n 1 input
                [ "$input" = $'\004' ] || [ "$input" = "q" ] && break
                echo -en "\b"
                main "$input"
            done
            echo -en "\b\b"
            ;;
        jukebox)
            ## Start a jukebox instance
            jukebox -n "$(hostname)" jukebox
            ;;
        toggle-video)
            ## Toggle video
            echo 'cycle vid' | socat - "$(mpvsocket)"
            ;;
        songs)
            ## Get all songs in the playlist, optionaly filtered by category
            ## Usage: m songs [cat]
            grep --color=auto -P '.+\t.+\t[0-9]+\t.*'"$2" "$PLAYLIST" |
                awk -F'\t' '{print $2" :: "$1}'
            ;;
        socket)
            ## Get the socket in use
            mpvsocket "${@:2}"
            ;;
        shuffle | shuf)
            ## Shuffle the playlist
            mpv_do '["playlist-shuffle"]' .error -r
            ;;
        r)
            ## Get help for interactive mode
            cat <<EOF
p: pause
c: current
k: volume up
j: volume down
H: previous chapter
L: next chapter
h: previous file
l: next file
J | u: seek backwards
K | i: seek forwards
r: interactive mode help
EOF
            ;;
        help)
            ## Get help
            if [ $# -gt 1 ]; then
                awk \ '
BEGIN                                          { in_main=0; in_case=0; print_docs=0; inner_case=-1 }
$0 ~ /main\(\)/                                { in_main=1 }
in_main && !inner_case && /([| ]'"$2"'\))|[| ]'"$2"'\s*\|.*\)/ { sub(/^ */, "", $0); print($0); print_docs=1 }
print_docs && /^\s+##.*/                       { sub(/^\s+##/, "", $0); print("\t"$0) }
/case/                                         { inner_case++ }
/esac/                                         { inner_case-- }
print_docs && !inner_case && /;;/              { print_docs=0 }
                ' "$0"
            else
                awk \ '
BEGIN                   {in_main=0; in_case=0;}
in_case && /^\s+##.*/   {sub(/^\s+##/, "", $0); print("\t"$0)}
in_case && /\s+[a-zA-Z][^)]*\)$/ {sub(/)/, "", $0); sub(/^ */, "", $0); print($0)}
in_main && /case/       {in_case=1}
$0 ~ /main\(\)/         {in_main=1}
' \
                    "$0"
            fi
            ;;
        *)
            error '¯\_(ツ)_/¯' "use r|help to see available commands"
            ;;
    esac
}

main "$@"
