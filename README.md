# m
A music player written accidentaly in bash using mpv as a """backend""".

## Dependencies
- dmenu
- fzf
- youtube-dl
- mpv
- notify-send
- dbus-send
- socat
- jq

## Usage

Use `m help` to get help on how to use the program.

This program is intended to be used with a playlist file localted at
`$XDG_CONFIG_HOME/m/playlist`.

**This file should not be edited by hand.**

Because I know someone will try, the format is as follows:
```
Song Name\tlink\ttime\tcategory1\tcategory2\t....
```

Another optional "config file" is a script that is intended to be to update a
status bar or something. It can be whatever you want as long as it's located at
`$XDG_CONFIG_HOME/m/update_panel.sh`

The following commands attempt to call this script at the end of their task:
- playlist
- pause
- quit
- vu (volume up)
- vd (volume down)
- prev
- next
- prev-file
- next-file

## "Tips and tricks"

This is intended to be used mostly as a way to have keybinds for your window
manager that control your music player.

## Instalation
```
cp path/to/m.sh /usr/bin/m
chmod +x /usr/bin/m
```

