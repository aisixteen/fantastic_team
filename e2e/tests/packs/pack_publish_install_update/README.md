# packs/pack_publish_install_update

Issue #42. A worker is packed, published to the local signed index, listed, installed as a template, and a newer version arrives as an opt-in update with a permission diff; then the installed version is taken down.

Free tier: no model call, no key, no network. The probe drives the `pack` and `pack_listing` agents through `fantastic_team send`, writing pack folders through the `files` agent (the harness grants the `send` leg that, `helpers/packs.py:wire_packs`).

Checks, in order: `pack` writes `pack.yaml` and the harness's Python content hash equals the binary's; `publish` signs (untrusted signer, so `list` skips it with a note, then lists it once trusted); `install` copies the template and creates no agent; v1.1.0 (one more port, one more grant) shows only with `opt_in`, with exactly the added port and grant, and installs nothing; `takedown` revokes locally (`pushed` false), the installed pack becomes a warning, `show` marks it revoked and refuses its install (`list` offers the newer, unrevoked version), `fetch` of its hash is refused before any download; an index edited without re-signing is skipped.
