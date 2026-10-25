# Install: zero to a first team

This guide installs `fantastic_team` from source. It covers what is in the repo on `main` today and nothing else.

## Status

- There are **no releases yet**. Build from source. Releases are tracked in #15 and #251.
- `fantastic` (the kernel) is a **git dependency** on branch `main` (see `Cargo.toml`). `Cargo.lock` pins the commit. There is no separate `fantastic_shift` binary in this repo.
- The GitHub App is registered with `fantastic_team setup github --name NAME [--org ORG]` (step 3). The by-hand steps there are the fallback.

## 1. Prerequisites

- Linux or macOS. CI unit-tests on Linux, macOS and Windows; the run was verified on Linux only.
- `git` and a Rust toolchain with `cargo`.
- `gh` (optional; the general build lets a worker run it, the GitHub build refuses it).

## 2. Build

    git clone https://github.com/aisixteen/fantastic_team
    cd fantastic_team
    sh scripts/build.sh

The binary is `target/release/fantastic_team`. This is the **general** build (Team), the default: workers get stock exec. The github connector is in it.

For the **GitHub** build (Team GitHub), run `sh scripts/build-github.sh`. It copies the tracked tree to a temporary folder, applies [patches/github-exec.patch](../patches/github-exec.patch) there, builds it and writes `target/github/release/fantastic_team-github`. In that build a worker cannot run forge CLIs, `git push` or credential commands; every GitHub action goes through the github connector. Your source tree is never changed.

Check it runs:

    ./target/release/fantastic_team --help

Output (verified on this box):

    usage: fantastic_team [--dir PATH] [--port N | --tui]
      no surface flag: fully headless (no port, no terminal UI)
      --port N: serve the web UI on 127.0.0.1:N
      --tui: the terminal UI (needs a terminal)
      (`mcp`, `send` and `readme` as the first word run fantastic's own commands)
      (`packs list|show|updates|publish|fetch|submit|report|takedown|page|pack|install|eval` as the first word sends a pack verb to a running team: see `pack_cli`)
      (`packs install` run from a terminal asks for the agent id of each port first; an empty answer skips it)
      packs list [--query Q] | packs show --name N [--version V] | packs updates [--opt-in true|false] | packs publish --pack DIR --url HTTPS_URL --signing-key FILE | packs fetch --name N --hash H --stage DIR [--version V] | packs submit --pack DIR --url HTTPS_URL --signing-key FILE | packs report --name N --version V --reason TEXT | packs takedown --name N --version V --signing-key FILE | packs page | packs pack --worker W --out DIR | packs install --pack DIR --into DIR [--worker W] [--timeout-s N] [--parent-id AGENT [--approve] [--allow-code]] | packs eval --pack DIR --worker W [--timeout-s N]

There is no `--version` flag.

## 2a. Reproducible builds

Both builds are reproducible in intent (#137): the same commit gives the same bytes on two machines. Not yet proven end to end, because the canvas zip inside the binary is not deterministic (see the note below).

- **Toolchain.** `rust-toolchain.toml` pins the exact Rust version. rustup installs it on first use; CI reads the same pin.
- **Commands.** `sh scripts/build.sh` and `sh scripts/build-github.sh` build with `--locked` and the environment from `scripts/repro-env.sh`: `SOURCE_DATE_EPOCH` (the last commit's time), `TZ=UTC`, `LC_ALL=C`, and `--remap-path-prefix` for the source tree, cargo's home and rustup's home. The GitHub build remaps its temporary copy to `/src` too. The target directory (`CARGO_TARGET_DIR`, or the default `target/`) is remapped to `/target`, so two builds with different target dirs give the same bytes. Each prints `built: <path>` for its versioned copy, `fantastic_team-<version>-<target>` and `fantastic_team-github-<version>-<target>`.
- **Checksums.** `sh scripts/checksums.sh <dir>` writes `SHA256SUMS`, `MD5SUMS` and `RELEASE-INFO.txt` (commit, patch name and sha256, toolchain, rustc and cargo, the fantastic commit from `Cargo.lock`).
- **Verify.** `sh scripts/verify.sh <binary>` checks the binary's SHA-256 against its `SHA256SUMS` line. Exit 0 means it matches; 1 means a mismatch or no entry for the name; 2 means a usage or file error. `sh scripts/verify.sh --rebuild <binary>` also rebuilds from this checkout (check out the tag first) and compares the hash.
- **SHA-256 and MD5.** Use SHA-256; it is the checksum to compare. `MD5SUMS` is only there for corruption checks on a mirror. MD5 is not a security check.
- **Signing is laptop-only.** No signing, notarization or key material runs in CI or sits in GitHub. A signature is made by the maintainer on a laptop, after the checksums.
- **CI.** The `repro` job runs with `-f e2e=true`. It builds the same commit twice, in two folders and two target directories, and compares `SHA256SUMS`. It is non-strict by default: a difference is a warning. `-f strict=true` makes a difference fail the job. Turn `strict` on once the canvas zip is deterministic.
- **The canvas zip.** fantastic's canvas packer writes its zip with the current file times and extra fields, so the zip embedded in the binary changes on every build. The exact changes that fix it belong in fantastic: see [docs/fantastic-notes.md](fantastic-notes.md). Until they land, the two builds differ and `repro` reports it.

## 3. Register the GitHub App

Only needed for the GitHub side (pull requests, issue comments, labels, reading the task queue). Register your own app with:

    ./target/release/fantastic_team setup github --name NAME [--org ORG]

It opens a local page that posts the app's manifest to GitHub, receives the redirect, and stores the app id and private key in the credential store. The key is never written to a file or printed. `--org` creates the app under an organization. The setup asks for exactly these permissions:

- Contents: read and write
- Pull requests: read and write
- Issues: read and write
- Checks: read
- Metadata: read
- Actions: read

By hand instead, store its `app_id` and its PEM `private_key` under one ref in fantastic's credential store, bound to `api_url` (default `https://api.github.com`). The key stays in the store. Never write it in a file, a config, or a doc.

## 4. Models and keys

Keys go in the credential store, never in a file. This repo does not define provider environment variable names, so none are listed here. The GitHub app key goes in the store as in step 3.

For remote boxes, `scripts/remote-start.sh` reads the values of the variable names passed in `--keys` on your laptop and pipes them to the remote shell over SSH stdin. They are never in argv, a file, or shell history.

## 5. Minimal config and start

The example config is `examples/team.yaml` (one master, one group, two workers). Start the team headless:

    ./target/release/fantastic_team --dir ./my-team

Serve the web UI on a port:

    ./target/release/fantastic_team --dir ./my-team --port 8123

Terminal UI:

    ./target/release/fantastic_team --dir ./my-team --tui

Run it on a remote box with the scripts (each prints its usage with `--help`; all support `--dry-run`):

- `scripts/remote-add.sh <name> <host>:<dir> [--bin PATH] [--port N] [--keys "NAME …"]`
- `scripts/remote-start.sh <name> [--port N]` (starts detached; the binary binds 127.0.0.1 only)
- `scripts/remote-attach.sh <name> [--port N] [--tui]`
- `scripts/remote-logs.sh <name> [-f] [-n LINES]`
- `scripts/remote-ls.sh`
- `scripts/remote-stop.sh <name> [--now] [--yes]`

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
| Secrets check fails on commit | `scripts/check-secrets.sh .` found a key or token. Remove it and store it in the credential store. |
| Remote start leaves no keys | Name the variables in `--keys` on `remote-add.sh`; `remote-start.sh` reads them on your laptop. |

## Tested on

Linux, this box: built with `cargo build -p fantastic-team --bin fantastic_team` (exit 0), ran `--help` (above), and ran `--help` on each `scripts/remote-*.sh`. Not run: `--port`, `--tui`, a live GitHub app, real keys. macOS: not yet verified.
