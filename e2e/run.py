#!/usr/bin/env python3
"""The e2e launcher for the team binary. Stdlib only.

    python3 e2e/run.py --only-free            every free test (no network, no key)
    python3 e2e/run.py --only-turns           the turns tier: a real model, by hand (skips without a key)
    python3 e2e/run.py --only-turns --dry-run discover the turns tests and collect their probes; start nothing
    python3 e2e/run.py --only task/one_issue_to_draft_pr
    python3 e2e/run.py --build github         the tests for the GitHub build (default: general)
    python3 e2e/run.py --selftest             prove the harness itself, no binary
    python3 e2e/run.py --bin PATH ...         use a built binary instead of building

A test is a folder `e2e/tests/<group>/<id>/` with `test.yaml` (`costs`, `timeout`,
`plug`, `build`), `team.yaml`, a plug's input (`script.json` for the scripted plug) and
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
import threading
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
from helpers import app as app_mod  # noqa: E402
from helpers import packs_pub as packs_mod  # noqa: E402
from helpers import registry as registry_mod  # noqa: E402
from helpers import run_flow  # noqa: E402
from helpers import team as team_mod  # noqa: E402
from helpers import web as web_mod  # noqa: E402
from helpers.verdict import Skip, Verdict  # noqa: E402

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
                        "plug": meta.get("plug", "scripted"),
                        "build": meta.get("build", "any")})
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

    def boot_team(self, env_extra: dict[str, str], port: int | None = None,
                  wire=None) -> team_mod.Team:
        self.team = team_mod.Team(self.binary, self.run_dir, env_extra)
        team_mod.boot(self.team, os.path.join(self.dir, "team.yaml"), self.vars, port)
        team_mod.wire(self.team, wire)
        return self.team

    def boot_packs(self, env_extra: dict[str, str], canvas: bool = False) -> team_mod.Team:
        """Boot a team that has no connectors (the packs tests): the same start-up, none of
        `wire()`'s grants for the credentials, puller, github and queue agents, and one
        grant of its own (the operator may call the `files` agent). `canvas` adds the canvas the
        app would make, so the pack browser panel is placed on it."""
        self.team = team_mod.Team(self.binary, self.run_dir, env_extra)
        team_mod.boot(self.team, os.path.join(self.dir, "team.yaml"), self.vars, canvas=canvas)
        packs_mod.wire_packs(self.team)
        return self.team


class Timeout(Exception):
    pass


def run_test(test: dict, binary: str, stamp_dir: str) -> str:
    """Run one test. Returns `PASS`, `FAIL` or `SKIP` (a probe raised `Skip`)."""
    name = test["name"]
    run_dir = os.path.join(stamp_dir, *name.split("/"))
    os.makedirs(run_dir)
    print(f"\n=== {name}  (costs: {test['costs']}, plug: {test['plug']})\n    run dir: {run_dir}")
    forge = model = ctx = None
    ok = False
    skipped = None
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
    except Skip as e:  # a resource this machine lacks: not a failure
        skipped = e.reason
    except Exception:  # a probe that raises is a failed test, with its trace
        traceback.print_exc()
    finally:
        if ctx and ctx.team:
            code = ctx.team.stop()
            print(f"  team stopped (exit {code})")
            if not ok and skipped is None:
                print("  --- team log (tail) ---\n" + ctx.team.log()[-3000:])
        for s in (model, forge):
            if s:
                s.stop()
    if skipped is not None:
        print(f"SKIP {name} ({skipped})")
        return "SKIP"
    print(f"--- {name}: {'PASS' if ok else 'FAIL'}")
    return "PASS" if ok else "FAIL"


def summary(results: dict) -> int:
    """The exit code of a run: non-zero when any test FAILed; a SKIP never fails it."""
    return 1 if any(r == "FAIL" for r in results.values()) else 0


def build() -> str:
    print("building fantastic_team, fantastic_team_app and the handle_serve example ...", flush=True)
    for cmd in (["-p", "fantastic-team", "--bin", "fantastic_team"],
                ["-p", "fantastic-team-app", "--bin", "fantastic_team_app"],
                ["-p", "fantastic-team", "--example", "handle_serve"]):
        if subprocess.run(["cargo", "build", *cmd], cwd=ROOT).returncode != 0:
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
        late = scripted.ScriptedModel([{"text": "a"}, {"text": "b"}]).start()
        try:
            threading.Timer(0.4, lambda: urllib.request.urlopen(urllib.request.Request(
                late.endpoint, data=b"{}", method="POST"), timeout=10).read()).start()
            n = run_flow.wait_model_quiet(late, quiet=1.0, limit=10)
            v.check("wait_model_quiet waits out a request that lands late", n == 1, str(n))
            v.check("and returns at once-ish when nothing comes", run_flow.wait_model_quiet(
                late, quiet=0.2, limit=5) == 1)
        finally:
            late.stop()
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
    sample = [{"name": n, "build": b} for n, b in
              [("a", "any"), ("b", "general"), ("c", "github")]]
    v.check("the build filter keeps any and the selected build",
            [t["name"] for t in for_build(sample, "general")] == ["a", "b"]
            and [t["name"] for t in for_build(sample, "github")] == ["a", "c"])
    names = {t["name"]: t["build"] for t in discover()}
    v.check("the exec tests name their build", names.get("exec/general_allows_gh") == "general"
            and names.get("exec/github_refuses_forge_calls") == "github")
    selftest_registry(v)
    selftest_relay(v)
    selftest_selection(v)
    selftest_turns(v)
    selftest_source(v)
    selftest_skip(v)
    selftest_scripted_thinking(v)
    selftest_web(v)
    selftest_team_args(v)
    selftest_app_helpers(v)
    selftest_new_tests_collect(v)
    return v.finish()


def selftest_registry(v: Verdict) -> None:
    """The fake hosted registry counts what a distrusting client does, and serves one that trusts it.
    The pack content hash is the binary's: the vector is checked against a real `pack` run in
    packs/pack_publish_install_update."""
    import ssl
    tmp = tempfile.mkdtemp(prefix="e2e-registry-")
    reg = registry_mod.FakeRegistry(tmp, {"/index.json": (200, '{"ok": true}')}).start()
    try:
        try:
            urllib.request.urlopen(reg.url + "/index.json", timeout=10,
                                   context=ssl.create_default_context())
            distrusted = False
        except (urllib.error.URLError, ssl.SSLError, OSError):
            distrusted = True
        deadline = time.time() + 5
        while reg.handshake_failures < 1 and time.time() < deadline:
            time.sleep(0.05)
        v.check("a client that does not trust the registry cert fails: a connection, a failed "
                "handshake, no request",
                distrusted and reg.connections >= 1 and reg.handshake_failures >= 1
                and reg.requests == [], f"{reg.connections} {reg.handshake_failures} {reg.requests}")
        trusting = ssl.create_default_context(cafile=reg.cert)
        with urllib.request.urlopen(reg.url + "/index.json", timeout=10, context=trusting) as r:
            body = r.read()
        v.check("a client that trusts the cert is served its route and the request is recorded",
                json.loads(body) == {"ok": True} and [q["path"] for q in reg.requests] == ["/index.json"])
    finally:
        reg.stop()
        shutil.rmtree(tmp, ignore_errors=True)
    got = packs_mod.content_hash({"b": "yz", "a": "x", "pack.yaml": "ignored"})
    v.check("content_hash follows the rule: sorted paths, NUL, u64 LE length, bytes, no pack.yaml",
            got == "9f52052b3bebf486794e5e47379af6aa002567bbad6ba7b0a90ce05191b62c3e", got)


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
        for _ in range(50):  # the relay books the usage as the stream closes: a moment after the reply
            if (a.usage_in, a.usage_out) == (10, 20) and a.served == 1:
                break
            time.sleep(0.1)
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


NEW_TESTS = ("app/launcher_start_attach_stop", "app/handle_serves_port", "app/window_per_platform",
             "app/attach_remote", "app/connector_setup", "dispatcher/chat_history_send_stream",
             "dispatcher/chat_features")


def selftest_skip(v: Verdict) -> None:
    """A probe that raises Skip is a SKIP: printed with its reason, never a failure."""
    import contextlib
    import io

    tmp = tempfile.mkdtemp(prefix="e2e-skip-")
    try:
        tdir = os.path.join(tmp, "t")
        os.makedirs(tdir)
        with open(os.path.join(tdir, "probe.py"), "w") as f:
            f.write("from helpers.verdict import Skip\n\ndef run(ctx):\n    raise Skip('no such thing here')\n")
        with open(os.path.join(tdir, "script.json"), "w") as f:
            f.write("[]")
        test = {"name": "g/skipper", "dir": tdir, "costs": "free", "timeout": 30, "plug": "scripted",
                "build": "any"}
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            res = run_test(test, "/nonexistent", os.path.join(tmp, "runs"))
        text = out.getvalue()
        v.check("a probe that raises Skip is reported as SKIP with its reason",
                res == "SKIP" and "SKIP g/skipper (no such thing here)" in text, text[-300:])
        v.check("a SKIP is not a failure, a FAIL is", summary({"a": "SKIP", "b": "PASS"}) == 0
                and summary({"a": "SKIP", "b": "FAIL"}) == 1)
        with open(os.path.join(tdir, "probe.py"), "w") as f:
            f.write("def run(ctx):\n    return 1\n")
        with contextlib.redirect_stdout(io.StringIO()):
            res = run_test(test, "/nonexistent", os.path.join(tmp, "runs2"))
        v.check("a probe that returns 1 is a FAIL", res == "FAIL")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_scripted_thinking(v: Verdict) -> None:
    """`thinking` streams a reasoning block before the text; `delay` holds the answer."""
    frames = scripted.stream_frames("the answer", thinking="some thought")
    raw = b"".join(frames)
    evs = [json.loads(line[6:]) for block in raw.decode().split("\n\n") for line in block.splitlines()
           if line.startswith("data: ")]
    starts = [(e["index"], e["content_block"]["type"]) for e in evs if e["type"] == "content_block_start"]
    deltas = [e["delta"] for e in evs if e["type"] == "content_block_delta"]
    v.check("a turn with thinking streams a thinking block (index 0) before the text block (index 1)",
            starts == [(0, "thinking"), (1, "text")], str(starts))
    v.check("the thinking arrives as thinking_delta, the text as text_delta",
            "".join(d.get("thinking", "") for d in deltas if d["type"] == "thinking_delta") == "some thought"
            and "".join(d.get("text", "") for d in deltas if d["type"] == "text_delta") == "the answer", str(deltas))
    v.check("parse_stream still returns the text only", scripted.parse_stream(raw) == "the answer")
    plain = [(e["index"], e["content_block"]["type"]) for e in
             (json.loads(line[6:]) for block in b"".join(scripted.stream_frames("x")).decode().split("\n\n")
              for line in block.splitlines() if line.startswith("data: "))
             if e["type"] == "content_block_start"]
    v.check("a turn without thinking is unchanged: one text block at index 0", plain == [(0, "text")], str(plain))
    m = scripted.ScriptedModel([{"text": "a", "delay": 0.4, "thinking": "t"}, {"text": "b"}]).start()
    try:
        def timed(i):
            t0 = time.time()
            req = urllib.request.Request(m.endpoint, data=b"{}", method="POST")
            with urllib.request.urlopen(req, timeout=10) as r:
                body = r.read()
            return time.time() - t0, body
        slow, body = timed(0)
        fast, _ = timed(1)
        v.check("a delayed turn answers after its delay, the next at once, with thinking in the stream",
                slow >= 0.4 and fast < 0.4 and b"thinking_delta" in body, f"{slow:.2f}s {fast:.2f}s")
    finally:
        m.stop()


def selftest_web(v: Verdict) -> None:
    """helpers/web.py against an in-process websocket server: masked frames of all three
    length forms round-trip, `event` frames are kept apart from replies, ids are unique."""
    import base64
    import hashlib
    import socket
    import struct
    import threading

    srv = socket.socket()
    srv.bind(("127.0.0.1", 0))
    srv.listen(4)
    port = srv.getsockname()[1]
    got: list[dict] = []

    def read_exact(c, n):
        b = b""
        while len(b) < n:
            d = c.recv(n - len(b))
            if not d:
                raise OSError("eof")
            b += d
        return b

    def frame(obj):
        data = json.dumps(obj).encode()
        n = len(data)
        if n < 126:
            head = bytes([0x81, n])
        elif n < 65536:
            head = bytes([0x81, 126]) + struct.pack("!H", n)
        else:
            head = bytes([0x81, 127]) + struct.pack("!Q", n)
        return head + data

    def serve():
        c, _ = srv.accept()
        req = b""
        while not req.endswith(b"\r\n\r\n"):
            req += c.recv(1)
        key = re.search(rb"Sec-WebSocket-Key: (\S+)", req).group(1)
        acc = base64.b64encode(hashlib.sha1(key + b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest())
        c.sendall(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                  b"Sec-WebSocket-Accept: " + acc + b"\r\n\r\n")
        try:
            while True:
                b0, b1 = read_exact(c, 2)
                n = b1 & 0x7F
                if n == 126:
                    n = struct.unpack("!H", read_exact(c, 2))[0]
                elif n == 127:
                    n = struct.unpack("!Q", read_exact(c, 8))[0]
                masked = bool(b1 & 0x80)
                mask = read_exact(c, 4)
                body = bytes(x ^ mask[i % 4] for i, x in enumerate(read_exact(c, n)))
                msg = json.loads(body)
                got.append({"masked": masked, "msg": msg})
                if msg.get("type") == "call":
                    c.sendall(frame({"type": "event", "src": "s", "payload": {"n": 1}}))
                    c.sendall(frame({"type": "reply", "id": msg["id"], "data": {"echo": msg["payload"]}}))
        except OSError:
            pass
        finally:
            c.close()

    threading.Thread(target=serve, daemon=True).start()
    ws = None
    try:
        ws = web_mod.WsClient(port)
        small = ws.call("t", {"x": 1})
        big_text = "z" * 300
        big = ws.call("t", {"text": big_text})
        huge_text = "y" * 70000
        huge = ws.call("t", {"text": huge_text}, timeout=30)
        v.check("a small, a 300-byte and a 70000-byte masked frame round-trip through the websocket",
                small == {"echo": {"x": 1}} and big == {"echo": {"text": big_text}}
                and huge == {"echo": {"text": huge_text}} and all(g["masked"] for g in got), str(small)[:80])
        v.check("event frames are kept apart from the reply", len(ws.events("s")) == 3
                and all(f.get("type") == "event" for f in ws.events("s")))
        ws.watch("lead")
        time.sleep(0.2)
        v.check("watch sends a watch frame the page's shim sends", got[-1]["msg"] == {"type": "watch", "src": "lead"})
        v.check("call ids are distinct, and tagged per connection", ws.call("t", {})  == {"echo": {}}
                and len({g["msg"].get("id") for g in got if g["msg"].get("type") == "call"}) == 4)
    finally:
        if ws:
            ws.close()
        srv.close()
    m = scripted.ScriptedModel([]).start()
    try:
        ok = web_mod.http_get(m.url + "/v1/models/x")[0]
        nf = web_mod.http_get(m.url + "/nope")[0]
        up_now = web_mod.up(m.port, "/v1/models/x")
    finally:
        m.stop()
    v.check("http_get returns the status of a 200 and of a 404; up() sees a server and refuses() a closed port",
            ok == 200 and nf == 404 and up_now and web_mod.refuses(m.port))
    v.check("encode_frame masks and uses the 16-bit and 64-bit lengths",
            web_mod.encode_frame(b"a" * 5)[1] == 0x80 | 5 and web_mod.encode_frame(b"a" * 200)[1] == 0x80 | 126
            and web_mod.encode_frame(b"a" * 70000)[1] == 0x80 | 127)


def selftest_team_args(v: Verdict) -> None:
    """Team.start passes extra_args after `--dir D`; boot's port goes to the final start only."""
    tmp = tempfile.mkdtemp(prefix="e2e-args-")
    t = None
    try:
        stub = os.path.join(tmp, "stub")
        with open(stub, "w") as f:
            f.write("#!/bin/sh\nprintf '%s\\n' \"$@\" > argv.txt\nmkdir -p .fantastic/cli\n"
                    ": > .fantastic/cli/mcp.sock\nexec sleep 20\n")
        os.chmod(stub, 0o755)
        t = team_mod.Team(stub, tmp, {})
        t.start(wait_s=10, extra_args=["--port", 18080])
        argv = open(os.path.join(tmp, "argv.txt")).read().split("\n")[:-1]
        v.check("Team.start puts extra_args after --dir", argv == ["--dir", tmp, "--port", "18080"], str(argv))
        v.check("Team.port is the port of the start", t.port == 18080)
        t.stop()
        os.remove(os.path.join(tmp, ".fantastic", "cli", "mcp.sock"))
        t.start(wait_s=10)
        argv = open(os.path.join(tmp, "argv.txt")).read().split("\n")[:-1]
        v.check("Team.start with no extra_args is [--dir D] only, and clears the port",
                argv == ["--dir", tmp] and t.port is None, str(argv))
        v.check("Team.argv builds the same command line", t.argv(["--port", 7]) == [stub, "--dir", tmp, "--port", "7"])
    finally:
        if t:
            t.stop()
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_app_helpers(v: Verdict) -> None:
    import socket

    p = app_mod.free_port()
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", p))
        bound = True
    except OSError:
        bound = False
    finally:
        s.close()
    v.check("free_port returns a port that can be bound", bound and 0 < p < 65536)
    tmp = tempfile.mkdtemp(prefix="e2e-app-")
    try:
        class C:
            binary = os.path.join(tmp, "fantastic_team")
        errs = []
        for fn in (app_mod.app_binary, app_mod.handle_example):
            try:
                fn(C())
            except Skip as e:
                errs.append(e.reason)
        v.check("app_binary and handle_example raise Skip when the file is not built next to --bin",
                len(errs) == 2 and "fantastic_team_app" in errs[0] and "handle_serve" in errs[1], str(errs))
        open(os.path.join(tmp, "fantastic_team_app"), "w").close()
        v.check("app_binary finds the file next to --bin", app_mod.app_binary(C()).endswith("fantastic_team_app"))
        parent = subprocess.Popen(["sh", "-c", "sleep 30 & wait"])
        try:
            kids: list[int] = []
            for _ in range(50):
                kids = app_mod.child_pids(parent.pid)
                if kids:
                    break
                time.sleep(0.1)
            v.check("child_pids lists the child of a process, and a process with none gives []",
                    len(kids) == 1 and app_mod.child_pids(kids[0]) == [], str(kids))
            v.check("alive sees it", app_mod.alive(kids[0]) if kids else False)
        finally:
            for k in app_mod.child_pids(parent.pid):
                os.kill(k, signal.SIGKILL)
            parent.kill()
            parent.wait()
        v.check("wait_gone sees a dead pid", bool(kids) and app_mod.wait_gone(kids[0], 5))
        os.makedirs(os.path.join(tmp, "secrets"))
        for rel, text in (("a.log", "xx NEEDLE yy"), ("secrets/k", "NEEDLE"), ("b.log", "clean")):
            with open(os.path.join(tmp, rel), "w") as f:
                f.write(text)
        v.check("files_holding finds a needle outside secrets/ and skips secrets/",
                app_mod.files_holding(tmp, "NEEDLE") == ["a.log"], str(app_mod.files_holding(tmp, "NEEDLE")))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_new_tests_collect(v: Verdict) -> None:
    """Every app/ and dispatcher/ test has its files, collects, and is in the free tier."""
    by = {t["name"]: t for t in discover()}
    v.check("the seven #37 e2e tests exist", set(NEW_TESTS) <= set(by), str(sorted(set(NEW_TESTS) - set(by))))
    bad = []
    for n in NEW_TESTS:
        t = by.get(n)
        if not t:
            continue
        ok, detail = collect_probe(t["dir"])
        files = all(os.path.isfile(os.path.join(t["dir"], f)) for f in ("README.md", "test.yaml", "team.yaml",
                                                                         "script.json", "probe.py"))
        if not (ok and files and t["costs"] == "free" and t["plug"] == "scripted" and t["build"] == "any"):
            bad.append(f"{n}: collect={detail} files={files} costs={t['costs']} plug={t['plug']} build={t['build']}")
    v.check("each collects, has its files, and is free / scripted / any build", not bad, "; ".join(bad))


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
    ap.add_argument("--build", choices=["general", "github"], default="general",
                    help="which build the binary is; runs the tests for that build (default general)")
    ap.add_argument("--list", action="store_true")
    a = ap.parse_args()

    if a.selftest:
        return selftest()
    tests = for_build(discover(), a.build)
    if a.only:
        missing = set(a.only) - {t["name"] for t in tests}
        if missing:
            sys.exit(f"no such test: {', '.join(sorted(missing))}")
    tests = pick(tests, a.only, a.only_free, a.only_turns)
    if a.list:
        for t in tests:
            print(f"{t['name']}\t{t['costs']}\t{t['plug']}\t{t['build']}")
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
    binary = os.path.abspath(a.bin) if a.bin else build()
    stamp_dir = os.path.join(HERE, "runs", time.strftime("%Y%m%d-%H%M%S"))
    results = {t["name"]: run_test(t, binary, stamp_dir) for t in tests}
    print("\n" + "\n".join(f"{r}  {n}" for n, r in results.items()))
    return summary(results)


if __name__ == "__main__":
    sys.exit(main())
