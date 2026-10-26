"""Helpers for the packs tests: build a worker folder, pack it, and drive the
`fantastic_team packs ...` CLI against the live team.

Everything is plain files under the run dir (the `files` agent is rooted there,
so every pack path is relative to the run dir). No verb, flag or record field is
added: the CLI maps flags onto the payloads the bundles already read; the two
payload fields with no flag (`signing_key`, `from_shifts`) go to the `pack` agent
with `Team.send`.
"""

from __future__ import annotations

import json
import os
import secrets
import subprocess

# One eval that always passes (the executor defaults a missing exit_code to 0) and
# one that always fails (exit_code 1 is never reported by a lead's reply).
PASS_EVAL = "- id: a\n  task: say ok\n  check:\n    exit_code: 0\n"
FAIL_EVAL = "- id: b\n  task: say ok\n  check:\n    exit_code: 1\n"


def write_worker(run_dir: str, name: str, evals_yaml: dict[str, str], blueprint: bool = False) -> str:
    """Write `workers/<name>/` (worker.json, evals/*.yaml, optionally a blueprint)."""
    root = os.path.join(run_dir, "workers", name)
    os.makedirs(os.path.join(root, "evals"), exist_ok=True)
    with open(os.path.join(root, "worker.json"), "w") as f:
        json.dump({"name": name, "version": "1.0.0", "model": "scripted-model",
                   "ports": [], "grants": []}, f)
    for fname, text in evals_yaml.items():
        with open(os.path.join(root, "evals", fname), "w") as f:
            f.write(text)
    if blueprint:
        bp = os.path.join(root, "blueprint")
        os.makedirs(os.path.join(bp, name), exist_ok=True)
        with open(os.path.join(bp, "blueprint.yaml"), "w") as f:
            f.write(f"agents:\n  - id: {name}\n    handler_module: ram_store.tools\n")
        with open(os.path.join(bp, name, "directive.md"), "w") as f:
            f.write("Summarise issues.")
    return os.path.join("workers", name)


def cli(team, *args: str, timeout: int = 120) -> tuple[int, dict]:
    """Run `fantastic_team packs <args>` in the run dir. Returns (exit code, the printed row)."""
    r = subprocess.run([team.binary, "packs", *args], env=team.env, cwd=team.dir,
                       capture_output=True, text=True, timeout=timeout)
    try:
        row = json.loads(r.stdout)
    except ValueError:
        row = {"raw": (r.stdout + r.stderr).strip()}
    return r.returncode, row


def make_pack(team, name: str, evals_yaml: dict[str, str], blueprint: bool = False) -> str:
    """Write a worker and pack it into `packs/<name>`. Returns the pack folder (relative)."""
    worker = write_worker(team.dir, name, evals_yaml, blueprint)
    rc, row = cli(team, "pack", "--worker", worker, "--out", f"packs/{name}")
    if rc != 0 or (row.get("result") or {}).get("error"):
        raise RuntimeError(f"cannot pack {name}: {rc} {row}")
    return f"packs/{name}"


def write_signing_seed(run_dir: str, name: str = "pack_signer") -> str:
    """A random 64-hex seed in secrets/ (gitignored). Returns its path relative to the
    run dir, the file bridge path the verbs take. The seed is never printed."""
    sdir = os.path.join(run_dir, "secrets")
    os.makedirs(sdir, exist_ok=True)
    gi = os.path.join(run_dir, ".gitignore")
    if not os.path.exists(gi) or "secrets/" not in open(gi).read():
        with open(gi, "a") as f:
            f.write("secrets/\n")
    with open(os.path.join(sdir, f"{name}.key"), "w") as f:
        f.write(secrets.token_hex(32))
    return f"secrets/{name}.key"


def result(row: dict) -> dict:
    """The verb's reply out of a `send` row."""
    return row.get("result") or {}


def grant(team, agent: str, extra: dict, field: str = "callable_by") -> None:
    """Add entries to `agent`'s callable_by (or can_call), keeping the ones it has
    (`update_agent` replaces the whole map, and `reflect` shows the current one)."""
    r = team.send(agent, {"type": "reflect"})
    by = dict((r.get("result") or {}).get(field) or {})
    if not by:
        raise RuntimeError(f"cannot read {agent}'s {field}: {r}")
    by.update(extra)
    r = team.send("kernel", {"type": "update_agent", "id": agent, field: by})
    if r.get("status") != "completed":
        raise RuntimeError(f"cannot wire {agent}: {r}")


def wire_packs(team) -> None:
    """The operator grants the harness' two-step boot leaves out (see e2e/README, item 1).

    The first start (no team.yaml) seeds a base tree with its own `files` and `user`;
    the second start finds them and keeps them as they are, so their `callable_by` does
    not name the pack agents. Add `pack` and `pack_listing` to `files`, and `pack` (the
    `import_subtree` verb only) to `user`. A fresh run of the product creates them with
    these grants (tree.rs)."""
    grant(team, "files", {"pack": "*", "pack_listing": "*"})
    grant(team, "user", {"pack": ["import_subtree"]})
