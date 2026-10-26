# packs/pack_install_roundtrip (free)

Team A packs a source worker (a record with a fake `api_key` and a `model` binding, a directive, a one-agent blueprint) with the `pack` agent. Team B installs the pack as a template. The scripted model is never called.

Asserts: `pack.yaml` and the listed files exist; neither the key nor the model binding is in any packed file, `worker.json` has neither key, `scripts/check-secrets.sh` is clean; B installs at `templates/scribe-1.0.0` with A's content hash and makes no agent; a pack with one appended line is refused with `content hash mismatch` and writes nothing; the same version twice is `version already installed`.

Mutation: make `strip_bindings` return its input, or drop the content-hash refusal in `install_with`, and the key and hash checks fail.
