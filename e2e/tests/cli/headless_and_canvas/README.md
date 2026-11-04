# cli/headless_and_canvas

The two surfaces of fantastic's command line, and the canvas as an agent's own subagent.

1. `fantastic_team --headless`: no screen and no port. The team runs with stdin on /dev/null and
   no terminal (the UI would need one), and its process (and children) listen on no TCP port. The
   probe imports a lead (blueprint) that may `create_agent` on the root; the scripted lead makes a
   `canvas_page.tools` agent under itself: the canvas is compiled in and registered, so an agent
   can create one with no `canvas` word. It does not start a web host.
2. `fantastic_team canvas --port N` in the same folder: the same records, now with a web host:
   `/canvas_page/file/index.html` answers 200 on N and N is a LISTEN port.

Skips (prints SKIP, exit 0) when neither `lsof` nor `/proc/net/tcp` is there to list listeners.

Mutation: start phase 1 with `canvas --port N` and "binds no TCP port" fails.
