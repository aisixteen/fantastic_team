"""Steps the pack tests share: write pack folders through the `files` agent, pack,
publish, trust a signer. Every function sends through the team's `send` leg and
returns the reply dict (`team.send` wraps it as {status, result}; these unwrap)."""

from __future__ import annotations

import hashlib
import json
import struct

SEED = "0f" * 32  # a fixed throwaway signing seed (64 hex characters)
SEED_PATH = "keys/seed"
LISTING_URL = "https://registry.example.invalid/{name}-{version}.json"


def result(reply: dict) -> dict:
    """The payload of a `send` reply: its `result`, or the reply itself."""
    r = reply.get("result", reply)
    return r if isinstance(r, dict) else {"value": r}


def wire_packs(team) -> None:
    """The operator's grant a team.yaml cannot say: the `send` leg (`mcp`) may call
    the `files` agent, so the probe can write and read pack folders."""
    r = team.send("kernel", {"type": "update_agent", "id": "files",
                             "callable_by": {"$self": "*", "$parent": "*", "pack": "*",
                                             "pack_listing": "*", "mcp": "*"}})
    if r.get("status") != "completed":
        raise RuntimeError(f"cannot wire files: {r}")
    # The first start of a fresh dir seeds its own `user` (see README, "Things the harness works
    # around" 1), so the tree's grant "the pack agent may call user.import_subtree" is not
    # written; the operator writes it.
    users = result(team.send("kernel", {"type": "list_agents"})).get("agents", [])
    user = next((a for a in users if a.get("id") == "user"), None)
    have = (user or {}).get("callable_by", {}).get("pack") if user else None
    if user is not None and have != "*" and "import_subtree" not in (have or []):
        cb = dict(user.get("callable_by") or {}, pack=sorted(set(have or []) | {"import_subtree"}))
        r = team.send("kernel", {"type": "update_agent", "id": "user", "callable_by": cb})
        if r.get("status") != "completed":
            raise RuntimeError(f"cannot wire user: {r}")


def put(team, path: str, text: str) -> dict:
    return result(team.send("files", {"type": "write", "path": path, "bytes": text}))


def read(team, path: str) -> dict:
    return result(team.send("files", {"type": "read", "path": path}))


def text_of(team, path: str) -> str | None:
    """A file's text, or None when it is absent."""
    r = read(team, path)
    if "error" in r:
        return None
    if "content" in r:
        return r["content"]
    return "\n".join(l.get("text", "") for l in r.get("lines", []))


def exists(team, path: str) -> bool:
    return text_of(team, path) is not None


def make_worker(team, name: str, version: str, ports=("@model",), directives=None,
                extra_files=None, folder=None) -> str:
    """Write `<folder>/worker.json` (default folder `workers/<name>`; also directives, evals,
    ...). Returns the folder."""
    base = folder or f"workers/{name}"
    rec = {"name": name, "version": version, "ports": list(ports)}
    put(team, f"{base}/worker.json", json.dumps(rec))
    for rel, text in (directives or {"main.md": "Answer briefly."}).items():
        put(team, f"{base}/directives/{rel}", text)
    for rel, text in (extra_files or {}).items():
        put(team, f"{base}/{rel}", text)
    return base


def pack(team, worker: str, out: str) -> dict:
    return result(team.send("pack", {"type": "pack", "worker": worker, "out": out}))


def write_seed(team, path: str = SEED_PATH, seed: str = SEED) -> None:
    put(team, path, seed)


def publish(team, pack_dir: str, url: str, grants=(), key: str = SEED_PATH) -> dict:
    return result(team.send("pack_listing", {"type": "publish", "pack": pack_dir, "url": url,
                                             "signing_key": key, "grants": list(grants)}))


def takedown(team, name: str, version: str, key: str = SEED_PATH) -> dict:
    return result(team.send("pack_listing", {"type": "takedown", "name": name,
                                             "version": version, "signing_key": key}))


def listing(team, verb: str, **fields) -> dict:
    return result(team.send("pack_listing", dict(fields, type=verb)))


def trust(team, signer_key: str) -> dict:
    return result(team.send("kernel", {"type": "update_agent", "id": "pack_listing",
                                       "trusted_keys": [signer_key]}))


def set_registry(team, url: str) -> dict:
    """Set `registry_url` on the listing record; an empty string unsets it."""
    return result(team.send("kernel", {"type": "update_agent", "id": "pack_listing",
                                       "registry_url": url}))


def content_hash(files: dict) -> str:
    """The pack content hash: sha256 over the files sorted by path (pack.yaml left out),
    each as its path, a NUL, its length (u64 little-endian) and its bytes."""
    h = hashlib.sha256()
    for path in sorted(p for p in files if p != "pack.yaml"):
        data = files[path]
        data = data.encode() if isinstance(data, str) else data
        h.update(path.encode() + b"\0" + struct.pack("<Q", len(data)) + data)
    return h.hexdigest()


def hand_pack(team, out: str, name: str, version: str, files: dict, ports=("@model",)) -> str:
    """Write a pack folder by hand, for a pack the `pack` verb would refuse (it has a scan
    finding). File texts end with no newline (the bridge does not return one). Returns the hash."""
    digest = content_hash(files)
    manifest = {"name": name, "version": version, "author": "e2e", "license": "MIT",
                "price": "free", "content_hash": digest, "ports": list(ports),
                "files": sorted(files)}
    for rel, text in files.items():
        put(team, f"{out}/{rel}", text)
    put(team, f"{out}/pack.yaml", json.dumps(manifest, indent=2))  # JSON is YAML
    return digest
