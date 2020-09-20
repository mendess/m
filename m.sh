#!/bin/bash
#shellcheck disable=SC2119
#shellcheck disable=SC2155

#shellcheck disable=SC1090
[ -e ~/.config/user-dirs.dirs ] && . ~/.config/user-dirs.dirs

CONFIG_DIR="${XDG_CONFIG_HOME:-~/.config/}/m"
PLAYLIST="$(realpath "$CONFIG_DIR/playlist")"
SCRIPT_NAME="$(basename "$0")"
TMPDIR="${TMPDIR:-/tmp}"
MUSIC_DIR="${XDG_MUSIC_DIR:-~/Music}"
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
    notify "$@" >&2
}

update_panel() {
    [ ! -e "$CONFIG_DIR/update_panel.sh" ] || sh "$CONFIG_DIR/update_panel.sh"
}

check_cache() {
    local PATTERN
    PATTERN=("$MUSIC_DIR"/*"$(basename "$1" | grep -Eo '.......$')"*)
    [[ -f "${PATTERN[0]}" ]] && echo "${PATTERN[0]}" || echo "$1"
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
    bold() { if [ -t 1 ]; then echo -en "\e[1m$1\e[0m"; else echo -en "$1"; fi; }
    red() { if [ -t 1 ]; then echo -en "\e[1m$1\e[0m"; else echo -en "$1"; fi; }
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
            red "Error:" >&2
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
    if [ "$1" != force ] && { [ "$(mpvsocket)" != "/dev/null" ] || [ -z "$DISPLAY" ]; }; then
        WITH_VIDEO=no
    else
        WITH_VIDEO=$(echo "no
yes" | selector -i -p "With video?")
    fi
}

play() {
    case $WITH_VIDEO in
        yes)
            mpv \
                --geometry=820x466 \
                "$LOOP_PLAYLIST" \
                --input-ipc-server="$(mpvsocket new)"
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
                command -v bspc &>/dev/null && bspc rule -a \* -o desktop=^10
                $TERMINAL \
                    --class m-media-player \
                    -e mpv \
                    --geometry=820x466 \
                    "$LOOP_PLAYLIST" \
                    --input-ipc-server="$(mpvsocket new)" \
                    --no-video \
                    "$@" &
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
    local modes="single
random
All
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
                exit 1
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
            local vids="$(songs_in_cat "$catg" | xargs)"
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
            cd "$MUSIC_DIR" || exit 1
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
            m queue "${final_list[@]:10}" --no-move
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

        [ -z "$videoId" ] && exit 1
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

    [ -z "$current_song" ] && exit 2

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
    echo "$(mpvsocket)_last_queue"
}

interpret_song() {
    case "$1" in
        --search=*)
            echo "ytdl://ytsearch:${1#*=}"
            ;;
        -*)
            error "Invalid option:" "$1"
            return 1
            ;;
        http*)
            # local n_titles="$(youtube-dl \
            #     --max-downloads 1 \
            #     --get-title "$1" \
            #     --quiet |
            #     wc -l)"
            # [ "$n_titles" -ne 1 ] &&
            #     error 'Invalid link:' "$1" &&
            #     echo "[$(date)] $1" >>"/$TMPDIR/.queue_fails"

            check_cache "$1"
            ;;
        *)
            if [ -z "$1" ]; then
                error 'Error queueing' 'Empty file name'
                return 1
            elif [ -e "$1" ]; then
                echo "$1"
            else
                local matches="$(awk -F'\t' '{print($1"\t"$2)}' "$PLAYLIST" |
                    grep -i "$1")"
                local link="$(echo "$matches" | cut -f2)"
                { {
                    [ -z "$link" ] && error "No song found"
                } || {
                    [ "$(echo "$link" | wc -l)" -gt 1 ] &&
                        error "Too many matches:" "$(echo "$matches" | cut -f1)"
                }; } && return 1

                check_cache "$link"
            fi
            ;;
    esac
    return 0
}

queue() {
    local targets=()
    while [ $# -gt 0 ]; do
        case "$1" in
            -n | --notify)
                notify=1
                ;;
            -r | --reset)
                notify "Reseting queue..."
                rm -f "$(last_queue)"
                local reseted=1
                ;;
            -m | --no-move)
                no_move=1
                ;;
            -s | --search)
                shift
                targets+=("ytdl://ytsearch:$1")
                ;;
            -c | --category)
                shift
                while read -r line; do
                    targets+=("$(check_cache "$line")")
                done < <(songs_in_cat "$1" | shuf)
                ;;
            --category=*)
                while read -r line; do
                    targets+=("$(check_cache "$line")")
                done < <(songs_in_cat "${1#*=}" | shuf)
                ;;
            *)
                local t
                t="$(interpret_song "$1")" &&
                    [ -n "$t" ] &&
                    targets+=("$t") ||
                    return 1
                ;;
        esac
        shift
    done
    [ "${#targets[@]}" -lt 1 ] &&
        [[ ! "$reseted" ]] &&
        error "No files to queue" &&
        return 1

    for file in "${targets[@]}"; do
        echo -n "Queueing song: '$file'... "
        mpv_do '["loadfile", "'"$file"'", "append"]' --raw-output .error
        if [[ "$no_move" ]]; then
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
        [ "$notify" = 1 ] && {
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
        [[ "$file" =~ (ytdl|http).* ]] && {
            preempt_download "$playlist_pos" "$file" &
            disown
        }

        if [ "$(jobs -p | wc -l)" -ge "$(nproc)" ]; then
            wait -n
        fi
    done
    wait
    [ ${#targets[@]} -ge 5 ] && queue --reset
    :
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
    youtube-dl "$link" \
        --format 'bestaudio[ext=m4a]' \
        --add-metadata \
        --output ~/.cache/queue_cache/'%(id)s.%(ext)s' || return

    local id="$(youtube-dl "$link" --get-id)" || return

    echo "i: $id"
    local filename=~/.cache/queue_cache/"$id.m4a"
    mpv_do '["loadfile", "'"$filename"'", "append"]' >/dev/null
    mpv_do '["playlist-remove", '"$queue_pos"']' >/dev/null
    local count=$(mpv_get playlist-count --raw-output '.data')
    mpv_do '["playlist-move", '$((count - 1))', '"$queue_pos"']' >/dev/null
    while [ "$(($(mpv_get playlist-pos --raw-output .data) - 5))" -lt "$queue_pos" ]; do
        sleep 10m
    done
    mpv_do '["loadfile", "'"$2"'", "append"]' >/dev/null
    mpv_do '["playlist-remove", '"$queue_pos"']' >/dev/null
    count=$(mpv_get playlist-count --raw-output '.data')
    mpv_do '["playlist-move", '$((count - 1))', '"$queue_pos"']' >/dev/null
    rm "$filename"
}

now() {
    local current start end
    current="$(mpv_get playlist-pos | jq .data)"
    start="$((current - 1))"
    case "$start" in
        -1 | 0) start="1" ;;
    esac
    end="$((start + "${1:-10}"))"
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

for i in range(11):
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
    [ -z "$url" ] && error "'$url' is not a valid link" && exit 1
    entry="$(grep "$url" "$PLAYLIST")" &&
        error "$entry already in $PLAYLIST" &&
        exit 1
    categories=$(echo "${@:2}" | tr '[:upper:]' '[:lower:]' | tr ' ' '\t' | sed -E 's/\t$//')
    [ -n "$categories" ] && categories="	$categories"
    notify 'getting title'
    title="$(youtube-dl --get-title "$1" | sed -e 's/(/{/g; s/)/}/g' -e "s/'//g")"
    [ "${PIPESTATUS[0]}" -ne 0 ] && error 'Failed to get title from output' && exit 1

    notify 'getting duration'
    duration="$(youtube-dl --get-duration "$1" |
        sed -E 's/(.*):(.+):(.+)/\1*3600+\2*60+\3/;s/(.+):(.+)/\1*60+\2/' |
        bc -l)"

    notify 'adding to playlist'
    echo "$title	$url	$duration$categories"
    echo "$title	$url	$duration$categories" >>"$PLAYLIST"
}

del_song() {
    num_results="$(grep -c -i "$*" "$PLAYLIST")"
    results="$(awk -F'\t' 'BEGIN {IGNORECASE = 1} $0 ~ /'"$*"'/ {print $1}' "$PLAYLIST")"
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
            ## Togle pause
            if pgrep spotify &>/dev/null; then
                spotify_toggle_pause
            else
                echo 'cycle pause' | socat - "$(mpvsocket)"
                update_panel
            fi
            ;;
        quit)
            ## Kill the most recent player
            echo 'quit' | socat - "$(mpvsocket)"
            update_panel
            ;;
        play)
            ## Play something
            ##      Usage: m play [options] link
            ## Options:
            ##      -s | --search  Search the song on youtube
            case "$2" in
                -s | --search)
                    local song="$(interpret_song "--search=$3")"
                    ;;
                '')
                    error 'Give me something to play'
                    exit 1
                    ;;
                *)
                    local song="$(interpret_song "$2")"
                    ;;
            esac
            with_video force
            [[ "$song" != *playlist* ]] && LOOP_PLAYLIST=""
            play "$song"
            ;;
        playlist | play-interactive)
            ## Interactively asks the user what songs they want to play
            ## from their playlist
            [ -e "$PLAYLIST" ] || touch "$PLAYLIST"
            [ ! -s "$PLAYLIST" ] && error "Playlist file emtpy" && exit 1
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
                -q | --queue) m queue "$3" ;;
            esac
            youtube-dl --get-id "$2" |
                sed 's|^|https://youtu.be/|' |
                while read -r l; do
                    notify "adding $l"
                    main add_song "$l" "${@:3}"
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
            ##     -r | --reset            Resets the queue fairness
            ##     -s | --search STRING    Searches youtube for the STRING
            ##     -n | --notify           Send a notification
            ##     -m | --no-move          Don't move in the playlist, keep it at the end
            ##     -c | --category STRING  Queue all songs in a category
            queue "${@:2}"
            ;;
        del | delete-song)
            ## Delete a passed song
            [ $# -gt 1 ] || exit 1
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
            ## Increase volume by ${2:-2}%
            echo "add volume ${2:-2}" | socat - "$(mpvsocket)"
            update_panel
            ;;
        j | vd)
            ## Decrease volume by ${2:-2}%
            echo "add volume -${2:-2}" | socat - "$(mpvsocket)"
            update_panel
            ;;
        H | prev)
            ## Previous chapter in a file
            echo 'add chapter -1' | socat - "$(mpvsocket)"
            update_panel
            {
                sleep 2
                update_panel
            } &
            ;;
        L | next)
            ## Next chapter in a file
            echo 'add chapter 1' | socat - "$(mpvsocket)"
            update_panel
            {
                sleep 2
                update_panel
            } &
            ;;
        h | prev-file)
            ## Go to previous file
            if pgrep spotify &>/dev/null; then
                spotify_prev
            else
                echo 'playlist-prev' | socat - "$(mpvsocket)"
            fi
            {
                sleep 2
                update_panel
            } &
            ;;
        l | next-file)
            ## Skip to the next file
            if pgrep spotify &>/dev/null; then
                spotify_next
            else
                echo 'playlist-next' | socat - "$(mpvsocket)"
            fi
            {
                sleep 2
                update_panel
            } &
            ;;
        J | back)
            ## Seek ${2:-10}s backward
            echo "seek -${2:-10}" | socat - "$(mpvsocket)"
            ;;
        K | frwd)
            ## Seek ${2:-10}s forward
            echo "seek ${2:-10}" | socat - "$(mpvsocket)"
            ;;
        int | interactive)
            ## Enter interactive mode
            while :; do
                read -r -n 1 input
                [ "$input" = "q" ] && break
                [ "$input" = "c" ] && echo
                main "$input"
                [ "$input" = "c" ] || echo -en "\b"
            done
            ;;
        jukebox)
            jukebox -n "$(hostname)" jukebox
            ;;
        toggle-video)
            echo 'cycle vid' | socat - "$(mpvsocket)"
            ;;
        songs)
            grep --color=auto -P '.+\t.+\t[0-9]+\t.*'"$2" "$PLAYLIST" |
                awk -F'\t' '{print $2" :: "$1}'
            ;;
        r)
            ## Get help for interactive mode
            echo -en "\b"
            grep -Po ' \w\|\w[^)]+\)' "$(command -v "$0")"
            ;;
        help)
            ## Get help
            if [ $# -gt 1 ]; then
                awk \ '
BEGIN                                          { in_main=0; in_case=0; print_docs=0; inner_case=-1 }
$0 ~ /main\(\)/                                { in_main=1 }
in_main && !inner_case && /'"$2"'[a-zA-Z| ]*)/ { sub(/^ */, "", $0); print($0); print_docs=1 }
print_docs && /^\s+##.*/                       { sub(/^\s+##/, "", $0); print("\t"$0) }
/case/                                         { inner_case++ }
/esac/                                         { inner_case-- }
print_docs && !inner_case && /;;/              { print_docs=0 }
                ' "$0"
            else
                awk \ '
BEGIN                   {in_main=0; in_case=0;}
$0 ~ /main\(\)/         {in_main=1}
in_main && /case/       {in_case=1}
in_case && /\w[^)]*\)$/ {sub(/)/, "", $0); sub(/^ */, "", $0); print($0)}
in_case && /^\s+##.*/   {sub(/^\s+##/, "", $0); print("\t"$0)}' \
                    "$0"
            fi
            ;;
        *)
            error '¯\_(ツ)_/¯' "use r|help to see available commands"
            ;;
    esac
}

main "$@"
