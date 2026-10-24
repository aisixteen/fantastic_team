# Fantastic Team app packaging

`app/tauri.conf.json` packages the Fantastic Team UI as a desktop app for
macOS, Windows and Linux. The window loads the local web UI on
`127.0.0.1` only; no frontend is bundled.

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
launcher prints the exit status and exits with code 1. The window itself is
not built yet (see Remaining below).

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

- The Tauri window crate, behind a `tauri` feature, built like
  `aisixteen/fantastic` `src/app`: tauri 2.12.1, opening the window on the
  launcher's `web_url`.
- In-process home kernel through `TeamHandle` once #87 merges (today the
  launcher runs the headless binary as a child process).
- Board and worker views as the home screen (#29, #36).
- GitHub app setup UI (#30).
- Remote attach through a tunnel (#21).
- Mobile.
- Signing and notarization stay on the maintainer's laptop.
- App end-to-end test (launch, window, quit) on each desktop platform.
