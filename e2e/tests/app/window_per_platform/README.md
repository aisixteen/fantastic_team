# app/window_per_platform (free tier, skips without a display)

The packaged window opens on the web UI and quitting it stops the headless child (the launcher `fantastic_team_app` built with `--features tauri`).

Needs three things, and is a `Skip` (printed as `SKIP app/window_per_platform (reason)`, exit 0) without them:
- a tauri build of the app, `E2E_APP_TAURI_BIN=/absolute/path/fantastic_team_app` (`cargo build -p fantastic-team-app --features tauri`);
- Linux with a display (`DISPLAY`, Xvfb is fine) and WebKitGTK;
- `xdotool`.

With them it boots a team, stops it, launches the app with `--port P --binary BIN --dir RUN`, asserts the web UI answers at the URL the app prints, that the app has one headless child and a window titled `Fantastic Team`; then `xdotool windowquit` asks the window to close, and the app must end, the child be gone and the port refuse connections.

Not run here: the real Tauri window on macOS, Windows and Linux (launch, window, quit) is the maintainer's, by hand (the unchecked lines "Run the Tauri window once for real" and "App end-to-end per platform" in #37). This file is the scripted Linux leg, written and wired into the free tier, never run on a display by the author. Known before it runs: until the launcher's readiness URL is fixed (see `app/launcher_start_attach_stop`), the app does not become ready against the real binary, so "the web UI answers" is expected to fail on the first real run.

Mutation: none to run here (it skips); on a display, make the probe not send the close and "closing the window ends the app" fails.
