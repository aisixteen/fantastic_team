#!/usr/bin/env python3
"""The e2e launcher for the team binary. Stdlib only.

    python3 e2e/run.py --only-free            every free test (no network, no key)
    python3 e2e/run.py --only-turns           the turns tier: a real model, by hand (skips without a key)
    python3 e2e/run.py --only-turns --dry-run discover the turns tests and collect their probes; start nothing
    python3 e2e/run.py --only task/one_issue_to_draft_pr
    python3 e2e/run.py --selftest             prove the harness itself, no binary
    python3 e2e/run.py --bin PATH ...         use a built binary instead of building

A test is a folder `e2e/tests/<group>/<id>/` with `test.yaml` (`costs`, `timeout`,
`plug`), `team.yaml`, a plug's input (`script.json` for the scripted plug) and
`probe.py`, whose `run(ctx)` returns the exit code of its `Verdict`.
A test whose plug has no key in the environment is skipped (`SKIP`), not failed.
Runs live under `e2e/runs/<stamp>/<group>/<id>/` (gitignored: they hold model output).
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import secrets
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import traceback
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)

from backends import PLUGS, available, provides_vars, scripted  # noqa: E402
from backends import anthropic as relay_mod  # noqa: E402
from helpers import forge as forge_mod  # noqa: E402
from helpers import team as team_mod  # noqa: E402
from helpers.verdict import Verdict  # noqa: E402

# A real-key shape, built from pieces so this file does not hold one.
KEY_SHAPE = re.compile("sk-" "ant-" r"[A-Za-z0-9_\-]{20,}")
TURNS_TESTS = ["turns/task/worker_finishes_small_spec", "turns/secrets/real_key_stays_in_x_api_key"]


# --- tests -------------------------------------------------------------------
def read_meta(path: str) -> dict:
    """test.yaml is flat `key: value` lines."""
    meta: dict = {}
    with open(path) as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if ":" in line:
                k, _, v = line.partition(":")
                meta[k.strip()] = v.strip()
    return meta


def discover() -> list[dict]:
    """Every folder under tests/ with a test.yaml. Its name is its path under tests/
    (`task/one_issue_to_draft_pr`, `turns/task/worker_finishes_small_spec`)."""
    out = []
    base = os.path.join(HERE, "tests")
    for dirpath, dirs, files in os.walk(base):
        dirs.sort()
        if "test.yaml" in files:
            meta = read_meta(os.path.join(dirpath, "test.yaml"))
            name = os.path.relpath(dirpath, base).replace(os.sep, "/")
            out.append({"name": name, "dir": dirpath,
                        "costs": meta.get("costs", "free"),
                        "timeout": int(meta.get("timeout", "180")),
                        "plug": meta.get("plug", "scripted")})
    return sorted(out, key=lambda t: t["name"])


def pick(tests: list[dict], only=(), only_free=False, only_turns=False) -> list[dict]:
    """The tests a command line selects. --only-free never selects a turns test."""
    if only:
        tests = [t for t in tests if t["name"] in only]
    if only_free:
        tests = [t for t in tests if t["costs"] == "free"]
    if only_turns:
        tests = [t for t in tests if t["costs"] == "turns"]
    return tests


def collect_probe(test_dir: str) -> tuple[bool, str]:
    """Import a probe without starting anything: it must define `run`."""
    path = os.path.join(test_dir, "probe.py")
    try:
        spec = importlib.util.spec_from_file_location("collect_" + os.path.basename(test_dir), path)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
    except Exception as e:  # a probe that does not import is not collected
        return False, f"import failed: {e}"
    if not callable(getattr(mod, "run", None)):
        return False, "probe.py defines no run(ctx)"
    return True, "run(ctx) defined"


class Ctx:
    """What a probe gets: the plug, the fake forge, the team, the run dir."""

    def __init__(self, test: dict, run_dir: str, binary: str, model, forge) -> None:
        self.test, self.dir, self.run_dir = test, test["dir"], run_dir
        self.model, self.forge, self.binary = model, forge, binary
        self.vars: dict = {}
        self.team: team_mod.Team | None = None
        self.secrets: dict = {}

    def boot_team(self, env_extra: dict[str, str]) -> team_mod.Team:
        self.team = team_mod.Team(self.binary, self.run_dir, env_extra)
        team_mod.boot(self.team, os.path.join(self.dir, "team.yaml"), self.vars)
        team_mod.wire(self.team)
        return self.team


class Timeout(Exception):
    pass


def run_test(test: dict, binary: str, stamp_dir: str) -> bool:
    name = test["name"]
    run_dir = os.path.join(stamp_dir, *name.split("/"))
    os.makedirs(run_dir)
    print(f"\n=== {name}  (costs: {test['costs']}, plug: {test['plug']})\n    run dir: {run_dir}")
    forge = model = ctx = None
    ok = False
    try:
        bare = forge_mod.make_bare(os.path.join(run_dir, "forge"))
        forge_mod.seed_clone(bare, os.path.join(run_dir, "clone"))
        forge = forge_mod.Forge(os.path.join(run_dir, "forge")).start()
        variables = {"run_dir": run_dir, "forge_url": forge.url, "forge_host": forge.host}
        model = PLUGS[test["plug"]](test["dir"], variables)
        variables.update(provides_vars(model))  # model_name: what team.yaml's `model:` takes
        model.start()
        ctx = Ctx(test, run_dir, binary, model, forge)
        ctx.vars = dict(variables, model_endpoint=model.endpoint, model_host=model.host)
        spec = importlib.util.spec_from_file_location("probe_" + name.replace("/", "_"),
                                                      os.path.join(test["dir"], "probe.py"))
        probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(probe)

        def on_alarm(*_):
            raise Timeout(f"{name} ran past its {test['timeout']}s timeout")

        signal.signal(signal.SIGALRM, on_alarm)
        signal.alarm(test["timeout"])
        try:
            ok = probe.run(ctx) == 0
        finally:
            signal.alarm(0)
    except Exception:  # a probe that raises is a failed test, with its trace
        traceback.print_exc()
    finally:
        if ctx and ctx.team:
            code = ctx.team.stop()
            print(f"  team stopped (exit {code})")
            if not ok:
                print("  --- team log (tail) ---\n" + ctx.team.log()[-3000:])
        for s in (model, forge):
            if s:
                s.stop()
    print(f"--- {name}: {'PASS' if ok else 'FAIL'}")
    return ok


def build() -> str:
    print("building fantastic_team ...", flush=True)
    r = subprocess.run(["cargo", "build", "-p", "fantastic-team", "--bin", "fantastic_team"],
                       cwd=ROOT)
    if r.returncode != 0:
        sys.exit("cargo build failed")
    meta = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT,
                          capture_output=True, text=True, check=True).stdout
    return os.path.join(json.loads(meta)["target_directory"], "debug", "fantastic_team")


def dry_run(tests: list[dict]) -> int:
    """Print each test and whether its probe collects. Starts nothing."""
    bad = 0
    for t in tests:
        ok, detail = collect_probe(t["dir"])
        note = "" if available(t["plug"]) else "  (no key: would SKIP)"
        print(f"{'COLLECT' if ok else 'FAIL'} {t['name']}  (costs: {t['costs']}, plug: {t['plug']})"
              f"{note}  {detail}")
        bad += not ok
    print(f"{len(tests) - bad}/{len(tests)} probes collect")
    return 1 if bad else 0


def holding(root: str, needle: str) -> list[str]:
    """Paths under `root` whose bytes hold `needle`."""
    hits = []
    for base, _, names in os.walk(root):
        for n in names:
            p = os.path.join(base, n)
            try:
                if needle.encode() in open(p, "rb").read():
                    hits.append(os.path.relpath(p, root))
            except OSError:
                pass
    return hits


def post_to(url: str, key: str) -> tuple[int, bytes]:
    body = json.dumps({"model": "selftest", "max_tokens": 8, "messages": []}).encode()
    req = urllib.request.Request(url, data=body, method="POST", headers={
        "x-api-key": key, "content-type": "application/json", "anthropic-version": "2023-06-01"})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


# --- the harness proves itself ----------------------------------------------
def selftest() -> int:
    v = Verdict("selftest")
    m = scripted.ScriptedModel([
        {"calls": [{"target": "git", "type": "status", "args": {"path": "/x", "list": ["a", "b"]}}]},
        {"text": "all done for {{who}}"}], {"who": "me"}).start()
    try:
        def post(i):
            req = urllib.request.Request(m.endpoint, data=json.dumps({"n": i}).encode(),
                                         headers={"x-api-key": "k-" + str(i)}, method="POST")
            try:
                with urllib.request.urlopen(req, timeout=10) as r:
                    return r.status, r.read()
            except urllib.error.HTTPError as e:
                return e.code, e.read()
        s1, b1 = post(1)
        t1 = scripted.parse_stream(b1)
        v.check("the plug streams a call turn that a plain parser decodes", s1 == 200 and
                t1.startswith('<call name="send">') and "<target_id>git</target_id>" in t1
                and "<list><item>a</item><item>b</item></list>" in t1, t1)
        s2, b2 = post(2)
        v.check("the plug fills placeholders", scripted.parse_stream(b2) == "all done for me")
        s3, _ = post(3)
        v.check("a request past the script is a 500", s3 == 500 and m.overrun == 1 and not m.consumed)
        v.check("the plug records bodies and keys", m.keys == ["k-1", "k-2", "k-3"]
                and [r["n"] for r in m.requests] == [1, 2, 3])
    finally:
        m.stop()
    tmp = tempfile.mkdtemp(prefix="e2e-selftest-")
    f = None
    try:
        forge_mod.make_bare(tmp)
        f = forge_mod.Forge(tmp).start()

        def call(method, path, body=None, headers=None):
            req = urllib.request.Request(f.url + path, method=method, headers=headers or {},
                                         data=None if body is None else json.dumps(body).encode())
            try:
                with urllib.request.urlopen(req, timeout=10) as r:
                    return r.status, r.read()
            except urllib.error.HTTPError as e:
                return e.code, e.read()
        st, body = call("POST", "/app/installations/42/access_tokens", {})
        v.check("the forge mints a token", st == 201 and json.loads(body)["token"] == f.token)
        v.check("the token is not in this source", f.token.startswith("ghs_") and len(f.token) == 40)
        st, _ = call("POST", f"/repos/{forge_mod.REPO}/pulls",
                     {"title": "t", "head": "h", "base": "main", "draft": True},
                     {"Authorization": "Bearer abc"})
        pulls = f.pulls()
        v.check("the forge records a pull request verbatim, with its Authorization header",
                st == 201 and len(pulls) == 1 and pulls[0]["json"]["head"] == "h"
                and pulls[0]["headers"]["authorization"] == "Bearer abc")
        st, _ = call("GET", f"/{forge_mod.REPO}.git/info/refs?service=git-receive-pack")
        v.check("git over HTTP is refused without the token", st == 401)
        st, body = call("GET", f"/repos/{forge_mod.REPO}/issues/1/events")
        v.check("the issue was labelled by the maintainer",
                json.loads(body)[0]["actor"]["login"] == forge_mod.MAINTAINER)
    finally:
        if f:
            f.stop()
        shutil.rmtree(tmp, ignore_errors=True)
    selftest_relay(v)
    selftest_selection(v)
    selftest_turns(v)
    selftest_source(v)
    return v.finish()


def selftest_relay(v: Verdict) -> None:
    """The anthropic plug, against a fake upstream whose usage is known (10 in, 20 out)."""
    key = "relay-key-" + secrets.token_hex(8)  # a key of this run only, never a literal
    names = ("E2E_ANTHROPIC_KEY", "ANTHROPIC_API_KEY", "E2E_UPSTREAM", "E2E_MAX_REQUESTS",
             "E2E_PRICE_IN_PER_MTOK", "E2E_PRICE_OUT_PER_MTOK")
    saved = {k: os.environ.get(k) for k in names}
    tmp = tempfile.mkdtemp(prefix="e2e-relay-")
    up = scripted.ScriptedModel([{"text": "a" * 80}] * 4).start()
    relays: list = []
    try:
        for k in names:
            os.environ.pop(k, None)
        os.environ["E2E_ANTHROPIC_KEY"] = key
        os.environ["E2E_UPSTREAM"] = up.url

        run_a = os.path.join(tmp, "a")
        os.makedirs(run_a)
        a = relay_mod.make(run_a, {"run_dir": run_a}).start()
        relays.append(a)
        s, body = post_to(a.endpoint, key)
        v.check("the relay streams the upstream's reply back unchanged",
                s == 200 and scripted.parse_stream(body) == "a" * 80, body[:200].decode(errors="replace"))
        v.check("the upstream got the team's x-api-key", up.keys[-1] == key, str(up.keys[-1:]))
        v.check("the recorded usage is the upstream's: 10 in, 20 out",
                (a.usage_in, a.usage_out) == (10, 20), f"{a.usage_in} in, {a.usage_out} out")
        v.check("one reply served, none errored: consumed", a.served == 1 and a.consumed)

        os.environ["E2E_PRICE_IN_PER_MTOK"] = "1"
        os.environ["E2E_PRICE_OUT_PER_MTOK"] = "2"
        a.stop()
        raw = open(os.path.join(run_a, "cost.json")).read()
        cost = json.loads(raw)
        v.check("cost.json records the usage, the model and the request count",
                cost["input_tokens"] == 10 and cost["output_tokens"] == 20
                and cost["model"] == a.model_name and cost["requests"] == 1, raw)
        v.check("cost.json prices the usage when both prices are set",
                abs(cost["usd_estimate"] - 0.00005) < 1e-9, raw)
        v.check("the forwarded key is never stored: cost.json and the run hold none",
                key not in raw and holding(run_a, key) == [], str(holding(run_a, key)))

        run_c = os.path.join(tmp, "cap")
        os.makedirs(run_c)
        os.environ["E2E_MAX_REQUESTS"] = "2"
        c = relay_mod.make(run_c, {"run_dir": run_c}).start()
        relays.append(c)
        codes = [post_to(c.endpoint, key)[0] for _ in range(3)]
        v.check("the (cap+1)th request gets 429, the first two are forwarded",
                codes == [200, 200, 429], str(codes))
        v.check("a refused request is not forwarded upstream", up.served == 3, f"upstream served {up.served}")
        v.check("a relay that refused a request is not consumed", not c.consumed)
    finally:
        for r in relays:
            r.stop()
        up.stop()
        for k, val in saved.items():
            if val is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = val
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_selection(v: Verdict) -> None:
    allt = discover()
    free = pick(allt, only_free=True)
    turns = pick(allt, only_turns=True)
    v.check("--only-free never selects a turns test",
            free and not any(t["costs"] == "turns" for t in free))
    v.check("--only-turns selects the turns tests and only them",
            {t["name"] for t in turns} == set(TURNS_TESTS), str([t["name"] for t in turns]))
    tmp = tempfile.mkdtemp(prefix="e2e-probe-")
    try:
        with open(os.path.join(tmp, "probe.py"), "w") as f:
            f.write("X = 1\n")  # no run(ctx)
        ok, detail = collect_probe(tmp)
        v.check("a probe without run() fails the collect check", not ok, detail)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_turns(v: Verdict) -> None:
    """Run the launcher as a child with no key in its environment: turns tests skip, exit 0."""
    env = {k: val for k, val in os.environ.items()
           if k not in ("E2E_ANTHROPIC_KEY", "ANTHROPIC_API_KEY")}
    me = os.path.abspath(__file__)
    r = subprocess.run([sys.executable, me, "--only-turns"], env=env, capture_output=True,
                       text=True, timeout=120)
    skips = all(f"SKIP {n} " in r.stdout for n in TURNS_TESTS)
    v.check("the turns tests skip without a key and the run exits 0",
            r.returncode == 0 and skips, (r.stdout + r.stderr)[-800:])
    r = subprocess.run([sys.executable, me, "--only-turns", "--dry-run"], env=env,
                       capture_output=True, text=True, timeout=120)
    collects = all(f"COLLECT {n} " in r.stdout for n in TURNS_TESTS)
    v.check("the turns tests collect under --dry-run without a key",
            r.returncode == 0 and collects, (r.stdout + r.stderr)[-800:])


def selftest_source(v: Verdict) -> None:
    """No file under e2e/ holds a real-key shape (the fake key is built from pieces)."""
    hits = []
    for base, dirs, names in os.walk(HERE):
        dirs[:] = [d for d in dirs if d not in ("runs", "__pycache__")]
        for n in names:
            if n.endswith((".py", ".yaml", ".yml", ".json", ".md", ".sh", ".txt")):
                p = os.path.join(base, n)
                with open(p, errors="replace") as f:
                    if KEY_SHAPE.search(f.read()):
                        hits.append(os.path.relpath(p, HERE))
    v.check("no source under e2e/ holds a real key (sk-ant- and 20+ key characters)", not hits,
            ", ".join(hits))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--only-free", action="store_true", help="run only tests whose costs is free")
    ap.add_argument("--only-turns", action="store_true",
                    help="run only tests whose costs is turns (a real model; by hand, never CI)")
    ap.add_argument("--dry-run", action="store_true",
                    help="discover the selected tests and collect their probes; start nothing")
    ap.add_argument("--only", action="append", default=[], metavar="GROUP/ID")
    ap.add_argument("--bin", help="a built fantastic_team (default: cargo build)")
    ap.add_argument("--selftest", action="store_true", help="check the harness, no binary")
    ap.add_argument("--list", action="store_true")
    a = ap.parse_args()

    if a.selftest:
        return selftest()
    tests = discover()
    if a.only:
        missing = set(a.only) - {t["name"] for t in tests}
        if missing:
            sys.exit(f"no such test: {', '.join(sorted(missing))}")
    tests = pick(tests, a.only, a.only_free, a.only_turns)
    if a.list:
        for t in tests:
            print(f"{t['name']}\t{t['costs']}\t{t['plug']}")
        return 0
    if not tests:
        sys.exit("no tests selected")
    if a.dry_run:
        return dry_run(tests)
    for t in tests:
        if not available(t["plug"]):
            print(f"SKIP {t['name']} (no key in the environment)")
    tests = [t for t in tests if available(t["plug"])]
    if not tests:
        print("nothing to run")
        return 0
    if shutil.which("git") is None:
        sys.exit("git is required")
    if not shutil.which("openssl"):
        print("openssl is absent: the tests need it to make a throwaway app key; skipping them")
        return 0
    if selftest() != 0:
        return 1
    binary = a.bin or build()
    stamp_dir = os.path.join(HERE, "runs", time.strftime("%Y%m%d-%H%M%S"))
    results = {t["name"]: run_test(t, binary, stamp_dir) for t in tests}
    print("\n" + "\n".join(f"{'PASS' if ok else 'FAIL'}  {n}" for n, ok in results.items()))
    return 0 if all(results.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
