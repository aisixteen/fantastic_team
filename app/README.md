# Fantastic Team app packaging

`app/tauri.conf.json` packages the Fantastic Team UI as a desktop app for
macOS, Windows and Linux. The window loads the local web UI on
`127.0.0.1` only; no frontend is bundled.

This is the general build (Team). The GitHub build (Team GitHub) is the same app with
`patches/github-exec.patch` applied: product name "AISixteen Fantastic Team GitHub",
identifier `com.aisixteen.team.github`, and a launcher that starts `fantastic_team-github`.

## The home screen

The dispatcher is the team's main view: a sidebar with the team's groups, and
in each group the workers on one level with their subagents. It is planned at
start (agent `dispatcher`, under the root) and placed on the canvas by the
first start only.

- Each worker is a row with its state (`busy`, `idle`, `stalled N min`,
  `done`, `failed`), the minutes on the task (`2m`, from `Row.minutes`), the
  number of its pull request (`#9`, from the board's PR link: the task in hand,
  else its latest done task with a PR) and its current task. Workers and subagents sit in one
  flat list per group: workers in normal weight with ● (busy) or ○, subagents
  smaller and muted with ◇.
- Each worker has a `subagents (N)` expander. It starts collapsed. Expanding
  one shows only that worker's subagents (its kernel agents), with the tool
  or kind and the last status. The expansion is held in the page and
  remembered in this browser per worker; it is not stored on the server.
- A click on a worker or a subagent opens that agent as a full-screen chat.
  The chat targets that agent alone and streams it live: it loads the
  history (the newest rows, oldest first), sends a message (shown at once as
  pending, then as sent; an error is shown on the error line and the message
  is removed), stops the turn, and draws the answer's text parts as they
  stream, until `done` re-enables Send. Back leaves the chat and unwatches
  it. Inline agent views are not in this slice (see Remaining).
- Back returns to the sidebar, restoring its scroll position and which
  workers were expanded.
- On a phone (430px wide and under) the sidebar stacks above the content.

**Controls.** Each worker row has Start, Pause, Stop, Hand task and Remove
buttons, and each group has an Add worker form (id and template). A button sends
`control {node, action, confirm}` to the dispatcher. The dispatcher accepts only
`start`, `pause`, `stop`, `hand_task`, `add_worker` and `remove_worker`; any other
action is refused and nothing is sent. Stop and Remove ask for confirmation first:
the team's question is shown in a dialog, and a no sends nothing. A refusal by the
node, or no reply from it, is shown as an error on the page, never as sent. The
sidebar reloads after each control. Subagents have no buttons.

- `hand_task {node: worker, task}` sends the worker `task <task>`; an empty task is
  refused.
- `add_worker {node: group, id, template}` makes the worker and its memory store
  from the template of `team.yaml` in the working directory (the file is read, not
  rewritten) and lists the worker in the group, in the dispatcher's `groups` and in
  the board's `team_groups`. The new worker gets its chat grant at the next start.
- `remove_worker {node: worker}` deletes the worker and its memory store and drops
  it from its group.

The dispatcher may call the root's `create_agent`, `update_agent` and
`delete_agent` for this (`tree::grant_agent_verbs` names it in the root's
`callable_by` at each start).

The load numbers (#127) reach the sidebar in its reply. A group with no AI
driver shows its raw counts under its title (`busy N · idle N · stalled N`); a
group with a driver shows nothing here, since its AI writes the summary.
The team load {tasks, done_last_hour} is drawn as a footer row under the tree:
`tasks: held N · in review N · done N · blocked N · queued N · done last hour N`.
It is left out when the board gave no load. When the cadencer answers, the footer
also shows the cost per hour: ` · digests N B/h · driver N tok/h`. Above the footer a
`waiting for you: N PRs · N stalled` line shows the PRs in review and the stalled
workers; stalled worker rows are marked.

The page talks to the dispatcher through fantastic's html connector
(`fantastic.send('dispatcher', …)`), with the `sidebar`, `open` and `control` verbs.
Served from a handle on a port, the page gets the same two calls from a small
shim over the team's `web_ws` leg (`team_ws`). Without
that connection it says so on one line instead of failing.

The chat talks to the agent itself, with the verbs and events fantastic's chat
cell uses: `history`, `send {text, client_id}`, `stop`, and `watch` for the
part events (`part_start`, `part_delta`, `part_end`, `part_abort`) and `done`
of this page's client. Only text parts are drawn. The page's agent
(`dispatcher_view`) may use those five verbs on each worker of a group and on
their subagents: `tree::grant_chat` writes both ends (the view's `can_call`, the
agent's `callable_by`) at each start, and no `*` is granted.

## Build locally

The launcher (`crates/team_app`, binary `fantastic_team_app`) starts the
headless `fantastic_team` binary for you and waits until its web UI answers:

- `fantastic_team_app` starts `fantastic_team --port 8765` (the port the
  packaged window loads) and prints `web UI at http://127.0.0.1:8765/`.
- `--port <n>` picks another port (1-65535; the window still loads 8765).
- `--binary <path>` runs a specific headless binary instead of one on `PATH`.
- `--dir <path>` passes `--dir` to the headless binary.
- `--attach <url>` attaches to a web UI already running on `127.0.0.1`,
  `localhost` or `[::1]` and starts nothing.

Ctrl-C stops the child. If the child exits before its web UI answers, the
launcher prints the exit status and exits with code 1.

**The window.** `cargo run -p fantastic-team-app --features tauri -- <flags>`
builds the launcher with the Tauri window (tauri 2.12.1, tauri-build 2.7.1,
built like `aisixteen/fantastic` `src/app`; needs Rust 1.90 or newer and, on
Linux, the webkit2gtk 4.1 and gtk3 dev packages). The flags are the same.
After the web UI answers, one window ("Fantastic Team", 1280x800) opens on the
launcher's `web_url` (the started port, or the `--attach` URL); it never opens
a URL that is not plain http on loopback. Closing the window stops the child;
a child that exits first ends the app with code 1. The build step reads
`app/tauri.conf.json` and writes its capability schemas to
`crates/team_app/gen/` (ignored by git). The `tauri` feature is off by
default: the plain build and `cargo test --workspace` stay tauri-free, and
the window spec (URL, title, size) is tested there without Tauri.

Build the packaged app from the repository root with the Tauri CLI:

- macOS (.app, .dmg): `cargo tauri build --bundles app,dmg`
- Windows (.msi, NSIS .exe): `cargo tauri build --bundles msi,nsis`
- Linux (.deb, AppImage): `cargo tauri build --bundles deb,appimage`

Artifacts land under `target/release/bundle/`.

## Signing and notarization (maintainer only)

Signing runs on the maintainer's laptop only. Nothing in this repository
stores an identity, team id, certificate, thumbprint or password.

- macOS: set the signing identity and notarization credentials in the
  maintainer's shell environment at build time, then build the dmg. The
  identifier is `com.aisixteen.team`.
- Windows: sign the msi/exe with the maintainer's code-signing certificate
  from the local certificate store.
- Linux: packages are unsigned by default; the maintainer adds a detached
  signature when publishing.

Never commit these values. The repository secret check (`scripts/check-secrets.sh`)
and `crates/team/tests/app_packaging.rs` guard against committed signing data.

## Remaining

Not in this slice:

- Sidebar, what is left (#122): inline agent views inside a chat (TBD per the
  issue, out of scope); per-worker expander persistence across restarts on the
  server side (today it is per browser).
- Chat, what is left (#37): the TS chat cell's features not ported: tool
  blocks, inquiry buttons, context stats, attachments, reasoning blocks (not
  drawn), and the `status` verb (a turn still streaming when the chat is
  reopened is not replayed). Grants reach only the workers in `team.yaml`
  groups and their subagents, as of the start; an agent made later is granted at
  the next start. No real browser check of the chat, and no live-canvas test.
  The stream handler is checked in CI with node (`dispatcher_page_chat_stream_draws_text_once`;
  it is skipped on a machine without node, and fails when `CI` is set).
- In-process home kernel through `TeamHandle` (#87 merged): `TeamHandle::start`
  with `TeamOptions.port` serves the dispatcher page on `127.0.0.1:<port>`
  (`TeamHandle::url()` names it: `/team_page/file/index.html`, since the host
  serves no `/`). The window still talks to the headless binary run as a child
  process; the host's record is removed at `stop`.
- No window run in CI (it needs a display and webkit), and CI does not build
  with `--features tauri`; `cargo check --features tauri` compiles, but the
  window has not been run yet (unverified). App end-to-end test per platform
  still open (below).
- Dispatcher, what is left (#29, #36): teams on other machines through the
  relay (#47, #49); connections and waiting-for-you panels from real data;
  start, pause and stop for a group or the whole team (the page controls one
  worker at a time); a worker made by add_worker is not written to `team.yaml`
  and gets the dispatcher's start/pause/stop/hand_task edge only once granted;
  the Tauri window loading the dispatcher instead of the
  canvas; the in-process kernel (#87); a real browser check of the page; a
  test that runs the team start with a live canvas page (the place step is
  covered only with a stand-in canvas).
- GitHub app setup UI (#30).
- Remote attach through a tunnel (#21).
- Mobile.
- Signing and notarization stay on the maintainer's laptop.
- App end-to-end test (launch, window, quit) on each desktop platform.
