
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
 - [ ] add-playlist       Append a playlist to the personal playlist
 - [ ] ch-cat             Add a category to the current song
     - depends on current

## Simple player interaction

 - [ ] frwd               Seek forward
 - [ ] back               Seek backward
 - [ ] next               Next chapter in a file
 - [ ] next-file          Skip to the next file
 - [ ] prev               Previous chapter in a file
 - [ ] prev-file          Previous file
 - [ ] loop               Toggles playlist looping
 - [ ] pause              Toggle pause
 - [ ] vd                 Volume up
 - [ ] vu                 Volume up
 - [x] shuffle            Shuffle
 - [ ] quit               Kill the most recent player
 - [ ] toggle-video       Toggle video

## Complex player interaction

 - [ ] current            Show the current song
 - [ ] now                Shows the current playlist

## Idk, it just depends on stuff before it

 - [ ] interactive        Enter interactive mode

## Queue management
 - [ ] play               Play something
 - [ ] queue              Queue a song
 - [ ] dequeue            Dequeue a song
 - [ ] dump               Save the playlist to a file to be restored later
 - [ ] load               Load a file of songs to play

## Other

 - [ ] lyrics             Shows lyrics for the current song
 - [ ] playlist           Interactively asks the user what songs they want to play from their playlist
