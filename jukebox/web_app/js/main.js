room_name = () => document.getElementById('room_name').value

show_error = (error) => document
  .getElementById('error')
  .innerHtml = `<p>${error}</p>`

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
  get('current', data => {
    document
      .getElementById("now-playing")
      .innerHTML = `<p>${data.replaceAll("\n", "<br>")}</p>`
    console.log(document.getElementById("now-playing").innerHTML)
  })
}

next_theme = () => {
  const themes = ["light", "dark"];
  next = (a) => themes[(themes.indexOf(a) + 1) % themes.length];
  Array.from(document.getElementsByClassName("themed"))
    .forEach(e => {
      for(let i = 0; i < themes.length; ++i) {
        if (e.className.indexOf(themes[i]) != -1) {
          e.className = e.className.replace(themes[i], next(themes[i]));
          break;
        }
      }
    });
}

window.onload = () => {
  document.getElementById("room_name").addEventListener(
    'keyup',
    ({key}) => { if (key === "Enter") now_playing() })
}
