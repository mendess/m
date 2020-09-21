const THEMES = ["light", "dark"];
let CURRENT_THEME = 0;
let PLAYLIST_LOADED = false;

window.onload = () => {
  document.getElementById("room_name").addEventListener(
    'keyup',
    ({key}) => { if (key === "Enter") now_playing() }
  );
  document.getElementById("queue_search").addEventListener(
    'keyup',
    event => filter_queue(event.target.value)
  );
}

filter_queue = (value) => {
  const filter = value
    .toUpperCase()
    .split(' ')
    .filter(e => e.trim().length > 0);
  const list = document.getElementsByClassName('playlist-li');
  let hidden_count = 0;
  for(let i = 0; i < list.length; ++i) {
    if (filter.every(s => list[i].innerText.toUpperCase().indexOf(s) > -1)) {
      list[i].style.display = ""
    } else {
      ++hidden_count;
      list[i].style.display = 'none';
    }
  }
  document
    .getElementsByClassName('playlist-li-default')[0]
    .style
    .display = hidden_count == list.length ? 'inherit' : 'none';

}

room_name = () => document.getElementById('room_name').value

show_error = (error) => document.getElementById('error').innerHTML = `${error}`

clear_error = () => document.getElementById('error').innerHTML = '';

api = (command, method) => fetch(`http://mendess.xyz:4193/${method}/${room_name()}/`,
  {
    method: 'POST',
    headers: {'Content-Type': 'application/json'},
    body: JSON.stringify({'cmd_line': command})
  }
)

run = (command) => {
  api(command, 'run')
    .then(response => {
      if (!response.ok) {
        show_error(response)
      }
    })
    .catch(show_error);
}

get = (command, action) => {
  api(command, 'get')
    .then(response => response.text())
    .then(action)
    .catch(show_error);
}

volume_up = () => { run('vu'); now_playing() }

volume_down = () => { run('vd'); now_playing() }

prev = () => { run('h'); now_playing() }

pause = () => { run('p'); now_playing(); }

next = () => { run('l'); now_playing() }

now_playing = () => {
  load_playlist();
  get('current', data => {
    console.log(data);
    document
      .getElementById("now-playing")
      .innerHTML = `<p>${data.replace(/\n/g, "<br>")}</p>`
  })
}

queue = (name) => {
  run(`queue "${name}"`);
  setTimeout(now_playing, 5000);
  document.getElementById('queue_search').value = '';
  filter_queue('')
}

search_queue = () => {
  const query = document.getElementById('queue_search').value;
  console.log('searching for', query);
  run(`queue -s "${query}"`);
  setTimeout(now_playing, 5000);
  document.getElementById('queue_search').value = '';
  filter_queue('')
}

next_theme = () => {
  let next = (CURRENT_THEME + 1) % THEMES.length;
  Array.from(document.getElementsByClassName("themed"))
    .forEach(e => e.className = e.className.replace(THEMES[CURRENT_THEME], THEMES[next]));
  CURRENT_THEME = next;
}

load_playlist = () => {
  if (PLAYLIST_LOADED) { return; }
  const list = document.getElementById("playlist")
  list.innerHTML = '';
  PLAYLIST_LOADED = true;
  get('songs', data => {
    data
      .split("\n")
      .map(l => l.split(' :: ')[1])
      .filter(l => l != undefined)
      .forEach(e => {
        list.innerHTML +=
          `<li class="playlist-li themed ${THEMES[CURRENT_THEME]}" onclick="queue('${e}')">${e}</li>`;
      });
    list.innerHTML = list.innerHTML.trim();
    if (list.innerHTML != '') {
      list.innerHTML +=
        `<li class="playlist-li-default themed ${THEMES[CURRENT_THEME]}" onclick="search_queue()">search on youtube</li>`;
    } else {
      PLAYLIST_LOADED = false;
    }
  })
}

