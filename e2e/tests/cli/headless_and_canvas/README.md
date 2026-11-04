# cli/headless_and_canvas

The surfaces of the command line (fantastic's `--headless` and `canvas`, the team's `webui`), and the canvas as an agent's own subagent.

1. `fantastic_team --headless`: no screen and no port. The team runs with stdin on /dev/null and
   no terminal (the UI would need one), and its process (and children) listen on no TCP port. The
   probe makes a lead with `create_agent` that may `create_agent` on the root itself; the scripted lead makes a
   `canvas_page.tools` agent under itself: the canvas is compiled in and registered, so an agent
   can create one with no `canvas` word. It does not start a web host.
2. `fantastic_team canvas --port N` in the same folder: the same records, now with a web host:
   `/canvas_page/file/index.html` answers 200 on N and N is a LISTEN port.

3. `fantastic_team webui --port N` in the same folder: `/team_page/file/index.html` answers 200 on N,
   N is a LISTEN port, `team_page` and `team_ws` are agents, `/team_ws/ws` accepts a websocket, and
   the binary says `the team page: <url>`.
4. `webui` with `canvas` is refused (exit 2, "cannot run together"): both mount a socket leg on the web
   host's one `/{host_id}/ws` route and the second would win.
5. `--headless` in the folder `webui` ran in opens no port, and the old port refuses connections.

Skips (prints SKIP, exit 0) when neither `lsof` nor `/proc/net/tcp` is there to list listeners.

Mutation: drop `webui::request()` from the binary's `main` and phase 3 fails; start phase 1 with `canvas --port N` and "binds no TCP port" fails.
