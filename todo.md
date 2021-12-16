
# Tasks

## Simple file io

 - [x] socket             Get the socket in use
 - [x] songs              Get all songs in the playlist, optionaly filtered by category
 - [x] cat                List all current categories
 - [ ] clean-downloads    Deletes downloaded songs that are not in the playlist anymore
     - [ ] decide on the file name format

## Playlist management

 - [x] new                Add a new song to the playlist
     - [ ] queue, depends on queue
 - [ ] delete-song        Delete a song from the playlist file
     - [ ] current, depends on current
 - [x] add-playlist       Append a playlist to the personal playlist
     - [ ] queue, depends on queue
 - [ ] ch-cat             Add a category to the current song
     - depends on current

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

 - [ ] current            Show the current song
 - [ ] now                Shows the current playlist

## Idk, it just depends on stuff before it

 - [ ] interactive        Enter interactive mode
     depends on current

## Queue management
 - [ ] play               Play something
 - [ ] queue              Queue a song
 - [ ] dequeue            Dequeue a song
 - [ ] dump               Save the playlist to a file to be restored later
 - [ ] load               Load a file of songs to play

## Other

 - [ ] lyrics             Shows lyrics for the current song
     - depends on current
 - [ ] playlist           Interactively asks the user what songs they want to play from their playlist

## After

- update lemons to use mlib
- implement events instead of using update_bar script
