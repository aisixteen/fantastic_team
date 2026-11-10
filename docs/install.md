# Install: zero to a first team

This guide installs `fantastic_team` from source. It covers what is in the repo on `main` today and nothing else.

## Status

- There are **no releases yet**. Build from source. Releases are tracked in #15 and #251.
- `fantastic` (the kernel) is a **git dependency** on branch `main` (see `Cargo.toml`). `Cargo.lock` pins the commit. There is no separate `fantastic_shift` binary in this repo.
- GitHub auth is configuration: a personal token from `GH_TOKEN`, or your own app by env refs (step 3).

## 1. Prerequisites

- Linux or macOS. CI unit-tests on Linux, macOS and Windows; the run was verified on Linux only.
- `git` and a Rust toolchain with `cargo`.
- `gh` (optional; a worker may run it).

## 2. Build

    git clone https://github.com/aisixteen/fantastic_team
    cd fantastic_team
    sh scripts/build.sh

The binary is `target/release/fantastic_team`. The github connector is in it.

Check it runs:

    ./target/release/fantastic_team readme

It prints fantastic's readme (the command line is fantastic's, unchanged). The team's own first words are listed in step 5. There is no `--version` flag and no `--help`: fantastic refuses an unknown word and prints its usage line.

There is no `--version` flag.

## 2a. Reproducible builds

The build is reproducible in intent (#137): the same commit gives the same bytes on two machines. Not yet proven end to end, because the canvas zip inside the binary is not deterministic (see the note below).

- **Toolchain.** `rust-toolchain.toml` pins the exact Rust version. rustup installs it on first use; CI reads the same pin.
- **Commands.** `sh scripts/build.sh` builds with `--locked` and the environment from `scripts/repro-env.sh`: `SOURCE_DATE_EPOCH` (the last commit's time), `TZ=UTC`, `LC_ALL=C`, and `--remap-path-prefix` for the source tree, cargo's home and rustup's home. The target directory (`CARGO_TARGET_DIR`, or the default `target/`) is remapped to `/target`, so two builds with different target dirs give the same bytes. It prints `built: <path>` for its versioned copy, `fantastic_team-<version>-<target>`.
- **Checksums.** `sh scripts/checksums.sh <dir>` writes `SHA256SUMS`, `MD5SUMS` and `RELEASE-INFO.txt` (commit, toolchain, rustc and cargo, the fantastic commit from `Cargo.lock`).
- **Verify.** `sh scripts/verify.sh <binary>` checks the binary's SHA-256 against its `SHA256SUMS` line. Exit 0 means it matches; 1 means a mismatch or no entry for the name; 2 means a usage or file error. `sh scripts/verify.sh --rebuild <binary>` also rebuilds from this checkout (check out the tag first) and compares the hash.
- **SHA-256 and MD5.** Use SHA-256; it is the checksum to compare. `MD5SUMS` is only there for corruption checks on a mirror. MD5 is not a security check.
- **Signing is laptop-only.** No signing, notarization or key material runs in CI or sits in GitHub. A signature is made by the maintainer on a laptop, after the checksums.
- **CI.** The `repro` job runs with `-f e2e=true`. It builds the same commit twice, in two folders and two target directories, and compares `SHA256SUMS`. It is strict by default: a difference fails the job; `-f strict=false` turns it into a warning.
- **The canvas zip.** fantastic's canvas packer writes its zip with the current file times and extra fields, so the zip embedded in the binary changes on every build. This is fixed in fantastic #409 (see [docs/fantastic-notes.md](fantastic-notes.md)); the pinned fantastic has it, and `tests/repro_scripts.sh` packs the canvas twice to check it.

## 3. GitHub auth

Only needed for the GitHub side (pull requests, issue comments, labels). The mode is configuration, set by env refs; nothing pastes or stores a token.

- Personal: export `GH_TOKEN` (a fine-grained token with Contents, Pull requests and Issues read/write, Metadata read). The stock `github_credentials` store reads it at each call; no command is needed.
- App: create and install your own app on GitHub by hand, with these permissions, then name it in team.yaml with `env: {app_id: VAR, private_key: VAR}` (the key variable holds the PEM text or the path of the .pem file):
  - Contents: read and write
  - Pull requests: read and write
  - Issues: read and write
  - Checks: read
  - Metadata: read
  - Actions: read
- Unset means local only: the team works with worktrees, branches and commits, and every GitHub verb says so.

Never write a value in a file, a config, or a doc.

## 4. Models and keys

Keys go in the credential store, never in a file. This repo does not define provider environment variable names, so none are listed here. The GitHub credential comes from env refs as in step 3.

## 5. Minimal config and start

The command line is fantastic's, unchanged: `fantastic_team` is `fantastic` with this repo's bundles added. It runs in the directory it is started in, so `cd` into the team's folder first. There is no config file: a team is agent records (a recipe, `send` with `create_agent`, or a directory that already holds `.fantastic/agents/`). The app makes a lead with the roles registry and the GitHub connector the same way (`crates/team/src/new_team.rs`).

    cd ./my-team
    ./target/release/fantastic_team --headless

No screen; the team page is served on a free 127.0.0.1 port.

Serve the web UI on a port:

    ./target/release/fantastic_team webui --port 8123

The terminal UI (needs a terminal):

    ./target/release/fantastic_team

`fantastic_team readme` prints fantastic's readme followed by the team's:

    fantastic_team readme

`--dir`, `--port` outside `canvas` and `webui`, `--tui`, `--instructions` and `--resend-instructions` are not flags any more; fantastic refuses them.

## 6. Label a first issue, watch, review

1. In the target repo, label an issue `team-ready`.
2. Add a comment that starts with `## SPEC` and holds the task. The puller takes it only when the latest label event is by the maintainer and the `## SPEC` comment is by the maintainer or the app.
3. The team's puller (`team_puller.tools`) reads it and writes `queue/<repo>-<n>.md` plus a pending `.state`. Its `status` verb shows progress.
4. The team opens a **draft** pull request. Review it on GitHub. Merging stays human.

## 7. Troubleshooting

| Symptom | Fix |
|---|---|
| `no ## SPEC comment by the maintainer or the app` | Add the `## SPEC` comment yourself, or let the app author it; comments by others are ignored. |
| Issue is not picked up | The latest `team-ready` label event must be by the maintainer. Re-add the label as the maintainer. |
| `gh --version` fails in the worker | Install `gh` and put it on PATH. |

## Tested on

Linux, this box: built with `cargo build -p fantastic-team --bin fantastic_team` (exit 0), and ran `readme` (above). Not run: `canvas --port`, the terminal UI, a live GitHub app, real keys. macOS: not yet verified.
