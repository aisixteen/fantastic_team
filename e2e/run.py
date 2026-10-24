#!/usr/bin/env python3
"""The e2e launcher for the team binary. Stdlib only.

    python3 e2e/run.py --only-free            every free test (no network, no key)
    python3 e2e/run.py --only task/one_issue_to_draft_pr
    python3 e2e/run.py --build github         the tests for the GitHub build (default: general)
    python3 e2e/run.py --selftest             prove the harness itself, no binary
    python3 e2e/run.py --bin PATH ...         use a built binary instead of building

A test is a folder `e2e/tests/<group>/<id>/` with `test.yaml` (`costs`, `timeout`,
`plug`, `build`), `team.yaml`, a plug's input (`script.json` for the scripted plug) and
`probe.py`, whose `run(ctx)` returns the exit code of its `Verdict`.
Runs live under `e2e/runs/<stamp>/<group>/<id>/` (gitignored: they hold model output).
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import shutil
import signal
import subprocess
import sys
import time
import traceback
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)

from backends import PLUGS, scripted  # noqa: E402
from helpers import forge as forge_mod  # noqa: E402
from helpers import team as team_mod  # noqa: E402
from helpers.verdict import Verdict  # noqa: E402


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
    out = []
    base = os.path.join(HERE, "tests")
    for group in sorted(os.listdir(base)):
        for tid in sorted(os.listdir(os.path.join(base, group))):
            d = os.path.join(base, group, tid)
            if os.path.exists(os.path.join(d, "test.yaml")):
                meta = read_meta(os.path.join(d, "test.yaml"))
                out.append({"name": f"{group}/{tid}", "dir": d,
                            "costs": meta.get("costs", "free"),
                            "timeout": int(meta.get("timeout", "180")),
                            "plug": meta.get("plug", "scripted"),
                            "build": meta.get("build", "any")})
    return out


def for_build(tests: list[dict], build: str) -> list[dict]:
    """The tests that run on a build: those that name no build (`any`) or that one."""
    return [t for t in tests if t["build"] in ("any", build)]


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
        model = PLUGS[test["plug"]](test["dir"], variables).start()
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
    import tempfile
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
    sample = [{"name": n, "build": b} for n, b in
              [("a", "any"), ("b", "general"), ("c", "github")]]
    v.check("the build filter keeps any and the selected build",
            [t["name"] for t in for_build(sample, "general")] == ["a", "b"]
            and [t["name"] for t in for_build(sample, "github")] == ["a", "c"])
    names = {t["name"]: t["build"] for t in discover()}
    v.check("the exec tests name their build", names.get("exec/general_allows_gh") == "general"
            and names.get("exec/github_refuses_forge_calls") == "github")
    return v.finish()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--only-free", action="store_true", help="run only tests whose costs is free")
    ap.add_argument("--only", action="append", default=[], metavar="GROUP/ID")
    ap.add_argument("--bin", help="a built fantastic_team (default: cargo build)")
    ap.add_argument("--selftest", action="store_true", help="check the harness, no binary")
    ap.add_argument("--build", choices=["general", "github"], default="general",
                    help="which build the binary is; runs the tests for that build (default general)")
    ap.add_argument("--list", action="store_true")
    a = ap.parse_args()

    if a.selftest:
        return selftest()
    tests = for_build(discover(), a.build)
    if a.only:
        tests = [t for t in tests if t["name"] in a.only]
        missing = set(a.only) - {t["name"] for t in tests}
        if missing:
            sys.exit(f"no such test: {', '.join(sorted(missing))}")
    if a.only_free:
        tests = [t for t in tests if t["costs"] == "free"]
    if a.list:
        for t in tests:
            print(f"{t['name']}\t{t['costs']}\t{t['plug']}\t{t['build']}")
        return 0
    if not tests:
        sys.exit("no tests selected")
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
