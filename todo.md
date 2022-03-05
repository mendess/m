
# Tasks

## Internal

 - [x] notify
 - [x] mpv_do
 - [x] selector
 - [ ] with_video
 - [x] check_cache
     - [x] download daemon
 - [x] partial name search

## Simple file io

 - [x] socket             Get the socket in use
 - [x] songs              Get all songs in the playlist, optionally filtered by category
 - [x] cat                List all current categories
 - [x] clean-downloads    Deletes downloaded songs that are not in the playlist anymore
     - [x] decide on the file name format
     - needs new id format to test
 - [x] status
     - now playing
     - queue size
     - last queue

## Playlist management

 - [x] new                Add a new song to the playlist
     - [x] queue, depends on queue
     - [x] search
 - [x] delete-song        Delete a song from the playlist file
     - [x] current, depends on current
     - needs new id format to test
 - [x] add-playlist       Append a playlist to the personal playlist
     - [x] queue, depends on queue
 - [x] ch-cat             Add a category to the current song
     - depends on current
     - needs new id format to test

## Simple player interaction

 - [x] frwd               Seek forward
 - [x] back               Seek backward
 - [x] next               Next chapter in a file
 - [x] next-file          Skip to the next file
 - [x] prev               Previous chapter in a file
 - [x] prev-file          Previous file
 - [x] loop               Toggles playlist looping
 - [x] pause              Toggle pause
 - [x] vd                 Volume up
 - [x] vu                 Volume up
 - [x] shuffle            Shuffle
 - [x] quit               Kill the most recent player
 - [x] toggle-video       Toggle video

## Complex player interaction

 - [x] current            Show the current song
 - [x] now                Shows the current playlist

## Idk, it just depends on stuff before it

 - [x] interactive        Enter interactive mode
     depends on current

## Queue management
 - [x] play               Play something
     - [x] handle long lists of files
 - [x] queue              Queue a song
     - [x] reset
     - [x] notify
     - [x] no_move
     - [x] clear
     - [x] category
     - [ ] preemptive download
 - [x] dequeue            Dequeue a song
 - [x] dump               Save the playlist to a file to be restored later
     depends on now
 - [x] load               Load a file of songs to play
     depends on now

## Other

 - [ ] lyrics             Shows lyrics for the current song
     - depends on current
 - [x] playlist           Interactively asks the user what songs they want to play from their playlist
 - [ ] info               Shows info on a random link or playlist item

## After

- update lemons to use mlib
- implement events instead of using update_bar script
