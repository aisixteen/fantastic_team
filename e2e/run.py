#!/usr/bin/env python3
"""The e2e launcher for the team binary. Stdlib only.

    python3 e2e/run.py --only-free            every free test (no network, no key)
    python3 e2e/run.py --only-turns           the turns tier: a real model, by hand (skips without a key)
    python3 e2e/run.py --only-turns --dry-run discover the turns tests and collect their probes; start nothing
    python3 e2e/run.py --only-gpu             the gpu tier: a vLLM server, by hand (skips without VLLM_ENDPOINT and its kin)
    python3 e2e/run.py --only-gpu --dry-run   discover the gpu tests and collect their probes; start nothing
    BACKEND=anthropic python3 e2e/run.py --only-gpu   the gpu tests on the anthropic relay instead (by hand; costs money)
    python3 e2e/run.py --only task/one_issue_to_draft_pr
    python3 e2e/run.py --selftest             prove the harness itself, no binary
    python3 e2e/run.py --bin PATH ...         use a built binary instead of building

A test is a folder `e2e/tests/<group>/<id>/` with `test.yaml` (`costs`, `timeout`,
`plug`), a plug's input (`script.json` for the scripted plug) and
`probe.py`, whose `run(ctx)` returns the exit code of its `Verdict`.
A `costs: gpu` test takes its plug from `BACKEND` (`vllm` or `anthropic`) when set; no other test
does. A test whose plug has no key (or, for vllm, no server variables) in the environment is skipped
(`SKIP`), not failed. `--only-free` never selects a turns or gpu test, and CI never runs them.
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

from backends import PLUGS, available, provides_vars, scripted, why_not  # noqa: E402
from backends import vllm as vllm_mod  # noqa: E402
from backends import anthropic as relay_mod  # noqa: E402
from helpers import forge as forge_mod  # noqa: E402
from helpers import app as app_mod  # noqa: E402
from helpers import cli as cli_mod  # noqa: E402
from helpers import fake_vllm  # noqa: E402
from helpers import podman_remote  # noqa: E402
from helpers import gpu_ssh  # noqa: E402
from helpers import ssh_twins  # noqa: E402
from helpers import run_flow  # noqa: E402
from helpers import stock as stock_mod  # noqa: E402
from helpers import team as team_mod  # noqa: E402
from helpers import web as web_mod  # noqa: E402
from helpers.verdict import Skip, Verdict  # noqa: E402

# A real-key shape, built from pieces so this file does not hold one.
KEY_SHAPE = re.compile("sk-" "ant-" r"[A-Za-z0-9_\-]{20,}")
TURNS_TESTS = ["turns/task/worker_finishes_small_spec", "turns/secrets/real_key_stays_in_x_api_key"]
CLI_SCENARIOS = ("single_lead", "lead_spawns_two", "precreated_1_2", "precreated_tree", "leads_spawn_leads")
# the gpu tier: the five cli scenarios (#237) and their ssh twins (#242)
GPU_TESTS = [f"{g}/{n}" for g in ("gpu_cli", "gpu_cli_ssh") for n in CLI_SCENARIOS]
SSH_TESTS = [f"cli_ssh/{n}" for n in CLI_SCENARIOS]
GIT_TWIN = f"cli_ssh/{ssh_twins.GIT_TWIN}"  # #253: the box's git credentials come through the bridge
BUSY_TWIN = f"cli_ssh/{ssh_twins.BUSY_TWIN}"  # #284: a lead's check-ins leave a generating sub-lead alone, stop an idle one


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
                        "plug": plug_of(meta),
                        "declared_plug": meta.get("plug", "scripted")})
    return sorted(out, key=lambda t: t["name"])


def plug_of(meta: dict) -> str:
    """A test's plug: its test.yaml's, except that a model test (costs gpu) takes `BACKEND`
    when set (`vllm` or `anthropic`), as fantastic picks its backend. A free test never does."""
    plug = meta.get("plug", "scripted")
    backend = os.environ.get("BACKEND")
    if backend and meta.get("costs") == "gpu":
        if backend not in ("vllm", "anthropic"):
            raise SystemExit(f"BACKEND must be vllm or anthropic, got {backend!r}")
        return backend
    return plug


def pick(tests: list[dict], only=(), only_free=False, only_turns=False, only_gpu=False) -> list[dict]:
    """The tests a command line selects. --only-free never selects a turns or gpu test."""
    if only:
        tests = [t for t in tests if t["name"] in only]
    if only_free:
        tests = [t for t in tests if t["costs"] == "free"]
    if only_turns:
        tests = [t for t in tests if t["costs"] == "turns"]
    if only_gpu:
        tests = [t for t in tests if t["costs"] == "gpu"]
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
        self.lead = "lead"  # the lead `stock.make_lead` creates, under a fixed id
        self.work = run_dir

    def boot_team(self, env_extra: dict[str, str] | None = None, **kw) -> team_mod.Team:
        """Start the stock team (`helpers/stock.py`): fantastic's CLI in the run folder, the
        the lead and the rest made with `send` + `create_agent`."""
        return stock_mod.boot(self, env_extra, **kw)


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
        variables.update(provides_vars(model))  # model_name: what the lead's `model` field takes
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
            if hasattr(model, "finish"):  # a plug that polls the agents' usage reads it before they go
                model.finish(ctx.team)
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
        why = why_not(t["plug"])
        note = "" if why is None else f"  (would SKIP: {why})"
        print(f"{'COLLECT' if ok else 'FAIL'} {t['name']}  (costs: {t['costs']}, plug: {t['plug']})"
              f"{note}  {detail}")
        bad += not ok
    print(f"{len(tests) - bad}/{len(tests)} probes collect")
    return 1 if bad else 0


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
    selftest_relay(v)
    selftest_fake_vllm(v)
    selftest_vllm_plug(v)
    selftest_selection(v)
    selftest_turns(v)
    selftest_gpu(v)
    selftest_backend(v)
    selftest_source(v)
    selftest_skip(v)
    selftest_scripted_thinking(v)
    selftest_web(v)
    selftest_team_args(v)
    selftest_scripted_lanes(v)
    selftest_cli_fixture(v)
    selftest_app_helpers(v)
    selftest_new_tests_collect(v)
    selftest_podman_remote(v)
    selftest_ssh_twins(v)
    selftest_remote_work(v)
    selftest_git_credentials(v)
    return v.finish()


def selftest_relay(v: Verdict) -> None:
    """The anthropic plug, against a fake upstream whose usage is known (10 in, 20 out)."""
    key = "relay-key-" + secrets.token_hex(8)  # a key of this run only, never a literal
    names = ("E2E_ANTHROPIC_KEY", "ANTHROPIC_API_KEY", "E2E_UPSTREAM", "E2E_MAX_REQUESTS",
             "E2E_PRICE_IN_PER_MTOK", "E2E_PRICE_OUT_PER_MTOK", "E2E_MODEL", "ANTHROPIC_CACHING",
             "E2E_WORKER_MODEL")
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
                key not in raw and app_mod.files_holding(run_a, key, skip=()) == [], str(app_mod.files_holding(run_a, key, skip=())))
        v.check("with no team watched, cost.json's `agents` is null", cost["agents"] is None, raw)
        relay_records(v, a, key)
        relay_usage(v, tmp, key)

        run_c = os.path.join(tmp, "cap")
        os.makedirs(run_c)
        os.environ["E2E_MAX_REQUESTS"] = "2"
        c = relay_mod.make(run_c, {"run_dir": run_c}).start()
        relays.append(c)
        codes = [post_to(c.endpoint, key)[0] for _ in range(3)]
        v.check("the (cap+1)th request gets 429, the first two are forwarded",
                codes == [200, 200, 429], str(codes))
        v.check("a refused request is not forwarded upstream", up.served == 3, f"upstream served {up.served}")
        v.check("a relay that refused a request is not consumed; it is capped, and says why",
                not c.consumed and c.capped and "E2E_MAX_REQUESTS" in (c.errored_reason or ""))
        run_d = os.path.join(tmp, "upstream_error")
        os.makedirs(run_d)
        os.environ.pop("E2E_MAX_REQUESTS")
        d = relay_mod.make(run_d, {"run_dir": run_d}).start()
        relays.append(d)
        codes = [post_to(d.endpoint, key)[0] for _ in range(2)]  # the upstream's script has one turn left
        v.check("an upstream error is passed on and counted, but is no cap: the gpu wait goes on",
                codes == [200, 500] and d.errored and not d.capped and d.errored_reason is None, str(codes))
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


VLLM_ENV = ("VLLM_ENDPOINT", "FANTASTIC_AI_MODEL", "VLLM_KV_SLOTS", "VLLM_KV_OVERFLOW", "VLLM_API_KEY",
            "MAX_TOKENS", "E2E_MAX_REQUESTS", "E2E_PRICE_IN_PER_MTOK", "E2E_PRICE_OUT_PER_MTOK")
#: every model variable: the vllm plug's, the anthropic plug's (its key names, model and caching) and
#: BACKEND, which points the gpu tests at a plug. A selftest child gets none of them (`child_env`).
MODEL_ENV = (*VLLM_ENV, "BACKEND", "E2E_MODEL", "E2E_WORKER_MODEL", "ANTHROPIC_CACHING", *relay_mod.KEY_ENVS)


def child_env(environ) -> dict:
    """The environment of a selftest's child run: the caller's, less every model variable. With one
    left, the child would run the real tests (and their selftest another child, and so on)."""
    return {k: val for k, val in environ.items() if k not in MODEL_ENV}


def relay_records(v: Verdict, relay, key: str) -> None:
    """The anthropic plug's AI records and recipe: what the gpu cli scenarios build with BACKEND=anthropic."""
    m = relay.model_name
    rec = relay.ai_record()
    v.check("the relay's AI record is an anthropic_backend one on the relay, caching on (provider_cache) by default",
            rec == {"handler_module": "anthropic_backend.tools", "model": m, "endpoint": relay.endpoint,
                    "caching": "provider_cache"} and m == relay_mod.DEFAULT_MODEL and key not in json.dumps(rec),
            str(rec))
    v.check("the recipe is `ai anthropic --model M --endpoint E`; update_agent carries the caching mode",
            relay.recipe_args() == ["--headless", "ai", "anthropic", "--model", m, "--endpoint", relay.endpoint]
            and relay.recipe_fields() == {"caching": "provider_cache"}, f"{relay.recipe_args()} {relay.recipe_fields()}")
    run_m = tempfile.mkdtemp(prefix="e2e-relay-rec-")
    made = []
    try:
        os.environ["E2E_MODEL"] = "claude-selftest"
        for mode in relay_mod.CACHING:
            os.environ["ANTHROPIC_CACHING"] = mode
            r = relay_mod.make(run_m, {"run_dir": run_m})
            made.append(r)
            v.check(f"ANTHROPIC_CACHING={mode} and E2E_MODEL are on the record and the recipe",
                    r.ai_record()["caching"] == mode and r.recipe_fields() == {"caching": mode}
                    and r.ai_record()["model"] == "claude-selftest" and "claude-selftest" in r.recipe_args(),
                    str(r.ai_record()))
        os.environ["ANTHROPIC_CACHING"] = "always"
        try:
            made.append(relay_mod.make(run_m, {"run_dir": run_m}))
            refused = False
        except RuntimeError:
            refused = True
        v.check("a caching mode that is not ai_core's is refused", refused)
        os.environ["ANTHROPIC_CACHING"] = "provider_cache"
        r = relay_mod.make(run_m, {"run_dir": run_m})
        made.append(r)
        same = cli_mod.worker_model() is None
        os.environ["E2E_WORKER_MODEL"] = "claude-worker-selftest"
        v.check("E2E_WORKER_MODEL is the model the lead is told to spawn with, the lead keeps E2E_MODEL; unset, "
                "a spawn names none",
                same and cli_mod.worker_model() == "claude-worker-selftest"
                and r.ai_record()["model"] == "claude-selftest", str(cli_mod.worker_model()))
    finally:
        os.environ.pop("E2E_WORKER_MODEL", None)
        os.environ.pop("E2E_MODEL", None)
        os.environ.pop("ANTHROPIC_CACHING", None)
        for r in made:
            r.httpd.server_close()
        shutil.rmtree(run_m, ignore_errors=True)


def relay_usage(v: Verdict, tmp: str, key: str) -> None:
    """The relay sums the anthropic agents' own `usage` (a fake team, no model) into cost.json."""
    import contextlib
    import io
    import types

    from helpers import gpu_cli as gpu_mod

    def usage(req, tin, cw, cr, tout, cost=None):
        u = {"requests": req, "input_tokens": tin, "cache_write_tokens": cw, "cache_read_tokens": cr,
             "output_tokens": tout, "prompt_tokens": tin + cw + cr, "saved_tokens": (18 * cr - 5 * cw) // 20, "hit_rate": 0.5}
        if cost is not None:
            u["cost_usd"] = cost
        return u
    run_u = os.path.join(tmp, "usage")
    os.makedirs(run_u)
    team = FakeUsageTeam({"lead": usage(2, 100, 400, 0, 30, 0.002), "w1": usage(1, 40, 0, 400, 10, 0.0005)},
                         handler="anthropic_backend.tools", others={"v1": "vllm_backend.tools"})
    r = relay_mod.make(run_u, {"run_dir": run_u}).start()
    try:
        r.poll_s = 0.05
        r.watch(team)
        end = time.time() + 5
        while time.time() < end and (r.agents_total() or {}).get("requests") != 3:
            time.sleep(0.05)
        v.check("the relay's poller reads every anthropic agent's usage while the run goes",
                (r.agents_total() or {}).get("requests") == 3, str(r.agents_total()))
        del team.counts["w1"]  # deleted by its parent: its last counts stay in the sum
        team.counts["lead"] = usage(4, 160, 400, 800, 50, 0.004)
        r.finish(team)
        a = r.totals()["agents"]
        v.check("the agents' block sums requests, input, cache write and read, output, prompt, saved and "
                "cost_usd, a deleted agent included",
                a == {"requests": 5, "input_tokens": 200, "cache_write_tokens": 400, "cache_read_tokens": 1200,
                      "output_tokens": 60, "prompt_tokens": 1800, "saved_tokens": 980, "cost_usd": 0.0045}, str(a))
        v.check("only the anthropic agents are asked for their usage",
                {tgt for tgt, p in team.sent if p.get("type") == "usage"} == {"lead", "w1"}, str(team.sent[:6]))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            gpu_mod.report(types.SimpleNamespace(model=r, team=team, elapsed=0))
        v.check("the gpu report prints the agents' block: cache write and read, cost_usd",
                "report: agents: 5 requests, 200 in, 400 cache write, 1200 cache read, 60 out, cost_usd 0.0045"
                in out.getvalue(), out.getvalue())
        team.counts["w2"] = usage(1, 10, 100, 0, 5)  # a record with no prices: its usage has no cost_usd
        r.finish(team)
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            gpu_mod.report(types.SimpleNamespace(model=r, team=team, elapsed=0))
        v.check("an agent with no prices makes the sum unpriced: cost_usd null, the report says so",
                r.agents_total()["cost_usd"] is None and "cost_usd unpriced" in out.getvalue(), out.getvalue())
        v.check("saved_tokens is each agent's latest reading, negative included (a cache write alone)",
                r.agents_total()["saved_tokens"] == 980 - 25, str(r.agents_total()))
        team.stop()
        r.finish(team)
        v.check("a stopped team is not asked again", r.agents_total()["requests"] == 6, str(r.agents_total()))
    finally:
        r.stop()
    raw = open(os.path.join(run_u, "cost.json")).read()
    cost = json.loads(raw)
    v.check("cost.json carries the agents' block beside the relay's own stream counts",
            cost["agents"] == r.agents_total() and cost["agents"]["cache_read_tokens"] == 1200
            and cost["requests"] == 0 and key not in raw, raw)


class vllm_env:
    """The vllm plug's variables for a block: set the given ones, clear the rest, restore after."""
    names = VLLM_ENV

    def __init__(self, **given: str) -> None:
        self.given = given

    def __enter__(self):
        self.saved = {k: os.environ.get(k) for k in self.names}
        for k in self.names:
            os.environ.pop(k, None)
        os.environ.update(self.given)

    def __exit__(self, *_):
        for k, val in self.saved.items():
            if val is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = val


class model_env(vllm_env):
    """Every model variable for a block (MODEL_ENV, and the relay's upstream): set the given ones,
    clear the rest, restore after."""
    names = (*MODEL_ENV, "E2E_UPSTREAM")


class FakeUsageTeam:
    """Stands in for a Team: AI agents of one backend that keep usage counts (the `usage` verb),
    agents of `others` (id -> handler) that have none, a log and a stop()."""

    def __init__(self, agents: dict, handler: str = "vllm_backend.tools", others: dict | None = None) -> None:
        self.counts = agents  # id -> the `usage` result
        self.handler = handler
        self.others = others or {}
        self.stopped = False
        self.proc = self
        self.sent: list[tuple[str, dict]] = []

    def poll(self):
        return 0 if self.stopped else None

    def agents(self) -> dict:
        out = {i: {"handler_module": self.handler} for i in self.counts}
        out.update({i: {"handler_module": h} for i, h in self.others.items()})
        out["credentials"] = {"handler_module": "credential_store.tools"}
        return out

    def log(self) -> str:
        return ""

    def send(self, target: str, payload: dict, timeout: float = 0) -> dict:
        self.sent.append((target, payload))
        if payload.get("type") == "usage" and target in self.counts:
            return {"status": "completed", "result": dict(self.counts[target])}
        return {"status": "failed", "result": {"error": "no such verb"}}

    def stop(self):
        self.stopped = True


def selftest_fake_vllm(v: Verdict) -> None:
    """The fake vLLM server answers the routes `vllm_backend` calls, in the shape it decodes."""
    srv = fake_vllm.FakeVllm(text="hello there", key="door-" + secrets.token_hex(6)).start()
    try:
        def call(method, path, key=None, body=None):
            req = urllib.request.Request(srv.url + path, method=method, data=body,
                                         headers={"Content-Type": "application/json",
                                                  **({"Authorization": f"Bearer {key}"} if key else {})})
            try:
                with urllib.request.urlopen(req, timeout=20) as r:
                    return r.status, r.read()
            except urllib.error.HTTPError as e:
                return e.code, e.read()

        chat = json.dumps({"model": "m", "messages": [{"role": "user", "content": "hi"}], "stream": True,
                           "stream_options": {"include_usage": True}}).encode()
        s401, _ = call("POST", "/v1/chat/completions", None, chat)
        s200, raw = call("POST", "/v1/chat/completions", srv.key, chat)
        got = fake_vllm.parse_chat_stream(raw)
        v.check("the fake vLLM wants the door's key: no Bearer is a 401, the key is a 200 stream",
                s401 == 401 and s200 == 200 and srv.refused == 1 and srv.auths == [f"Bearer {srv.key}"],
                f"{s401} {s200} {srv.auths}")
        v.check("the stream is a vLLM one: text, a stop, the usage-only frame, [DONE]",
                got["text"] == "hello there" and got["finish"] == "stop" and got["done"]
                and got["usage"]["prompt_tokens"] >= 1 and got["usage"]["completion_tokens"] >= 1, str(got))
        smodels, mraw = call("GET", "/v1/models", srv.key)
        listed = json.loads(mraw)["data"][0]
        v.check("GET /v1/models lists the served model with its max_model_len",
                smodels == 200 and listed["id"] == fake_vllm.MODEL and listed["max_model_len"] > 0, str(listed))
        v.check("it counts the requests and tokens it served",
                srv.served == 1 and srv.prompt_tokens == got["usage"]["prompt_tokens"], str(srv.served))
    finally:
        srv.stop()
    scripted_srv = fake_vllm.FakeVllm(script=[{"text": "first", "thinking": "hm"}, {"text": "second"}]).start()
    try:
        def ask():
            req = urllib.request.Request(scripted_srv.url + "/v1/chat/completions", method="POST", data=json.dumps(
                {"model": "m", "messages": [{"role": "system", "content": "sys"}, {"role": "user", "content": "q"}]}).encode())
            with urllib.request.urlopen(req, timeout=20) as r:
                return fake_vllm.parse_chat_stream(r.read())
        a, b = ask(), ask()
        v.check("a script plays through the fake in order, the thinking on the reasoning channel",
                (a["text"], a["reasoning"], b["text"]) == ("first", "hm", "second"), f"{a} {b}")
        try:
            ask()
            exhausted = False
        except urllib.error.HTTPError as e:
            exhausted = e.code == 500
        v.check("a request past the end of the script is a 500", exhausted)
    finally:
        scripted_srv.stop()
    lanes = scripted.ScriptedModel([{"text": "m", "lane": "main"}, {"text": "w", "lane": "w1"}])
    try:
        body = json.dumps({"messages": [{"role": "system", "content": "[lane:w1] in the system text"},
                                        {"role": "user", "content": "task [lane:w1]"}]})
        v.check("a lane is read off the first non-system message (an OpenAI body opens with the system one)",
                lanes.lane_of(body) == "w1")
    finally:
        lanes.httpd.server_close()


def selftest_vllm_plug(v: Verdict) -> None:
    """The vllm plug: what it needs, what its records say, what it counts and its cap."""
    with vllm_env():
        v.check("with none of its variables the plug cannot run, and says which are missing",
                not vllm_mod.available() and all(n in vllm_mod.problem() for n, _ in vllm_mod.REQUIRED),
                str(vllm_mod.problem()))
    base = {"VLLM_ENDPOINT": "https://gpu.example.invalid/v1", "FANTASTIC_AI_MODEL": "m-1",
            "VLLM_KV_SLOTS": "2", "VLLM_KV_OVERFLOW": "evict"}
    for gone in base:
        with vllm_env(**{k: val for k, val in base.items() if k != gone}):
            v.check(f"without {gone} the plug cannot run (the test SKIPs)",
                    not vllm_mod.available() and gone in vllm_mod.problem(), str(vllm_mod.problem()))
    for bad, why in (({"VLLM_KV_SLOTS": "3"}, "3 slots: the shared GPU keeps at most 2"),
                     ({"VLLM_KV_SLOTS": "0"}, "0 slots"), ({"VLLM_KV_SLOTS": "two"}, "slots that are no number"),
                     ({"VLLM_KV_OVERFLOW": "drop"}, "an overflow policy that is neither evict nor reject"),
                     ({"VLLM_ENDPOINT": "gpu.example.invalid"}, "an endpoint with no scheme")):
        with vllm_env(**{**base, **bad}):
            v.check(f"{why} is refused", not vllm_mod.available(), str(vllm_mod.problem()))
    with vllm_env(**base):
        v.check("all four variables, 2 slots: the plug can run", vllm_mod.available(), str(vllm_mod.problem()))
        v.check("why_not names nothing for it, and the launcher's table agrees", why_not("vllm") is None
                and available("vllm") and why_not("scripted") is None)
    tmp = tempfile.mkdtemp(prefix="e2e-vllm-")
    try:
        key = "door-" + secrets.token_hex(8)  # a key of this run only
        with vllm_env(**base, VLLM_API_KEY=key, MAX_TOKENS="4096", E2E_MAX_REQUESTS="5"):
            plug = vllm_mod.make(tmp, {"run_dir": tmp}).start()
            rec = plug.ai_record()
            runner = vllm_mod.runner_params()
            v.check("the AI record is a vllm_backend one that points straight at the server, with the slots, "
                    "the overflow policy and the output cap",
                    rec["handler_module"] == "vllm_backend.tools" and rec["endpoint"] == base["VLLM_ENDPOINT"]
                    and rec["model"] == "m-1" and rec["kv_slots"] == 2 and rec["kv_overflow"] == "evict"
                    and rec["max_tokens"] == 4096, str(rec))
            v.check("the sampling knobs are the ones beside the plug (the CoT-repetition fix)",
                    runner.get("no_repeat_ngram_size") == 3 and runner.get("repetition_penalty") == 1.05
                    and rec["no_repeat_ngram_size"] == 3 and "thinking_token_budget" not in rec, str(rec))
            v.check("the plug is keyed by the NAME of the variable; the credential ref is bound to the endpoint's host",
                    plug.key_env == "VLLM_API_KEY" and plug.keyed and plug.host == "gpu.example.invalid"
                    and plug.real and key not in json.dumps(rec))
            ra = plug.recipe_args()
            v.check("the recipe is `ai vllm -p PORT --host H --kv-overflow O`; update_agent carries the rest",
                    ra == ["--headless", "ai", "vllm", "-p", "443", "--host", "gpu.example.invalid",
                           "--kv-overflow", "evict"] and plug.recipe_fields()["endpoint"] == base["VLLM_ENDPOINT"]
                    and plug.recipe_fields()["kv_slots"] == 2 and "handler_module" not in plug.recipe_fields(),
                    f"{ra} {plug.recipe_fields()}")
            plug.stop()

            # usage, as the agents keep it; the poller sums it, a deleted agent keeps its last counts
            team = FakeUsageTeam({"lead": {"requests": 2, "prompt_tokens": 100, "output_tokens": 30},
                                  "w1": {"requests": 1, "prompt_tokens": 40, "output_tokens": 10}})
            plug = vllm_mod.make(tmp, {"run_dir": tmp})
            plug.poll_s = 0.05
            plug.watch(team)
            end = time.time() + 5
            while time.time() < end and plug.requests < 3:
                time.sleep(0.05)
            del team.counts["w1"]  # the worker was deleted by its parent
            team.counts["lead"]["requests"] = 4
            team.counts["lead"]["prompt_tokens"] = 160
            plug.finish(team)
            t = plug.totals()
            v.check("the plug sums the usage of every vllm agent, a deleted one included, and asks only them",
                    (t["requests"], t["input_tokens"], t["output_tokens"]) == (5, 200, 40)
                    and all(tgt in ("lead", "w1") for tgt, p in team.sent if p.get("type") == "usage"),
                    str(t))
            v.check("below the cap the run goes on", not plug.capped and not team.stopped and not plug.errored)
            team.counts["lead"]["requests"] = 6  # one past E2E_MAX_REQUESTS=5
            end = time.time() + 5
            while time.time() < end and not team.stopped:
                time.sleep(0.05)
            v.check("past E2E_MAX_REQUESTS the plug stops the team: the run ends, errored",
                    team.stopped and plug.capped and plug.errored and not plug.consumed, str(plug.totals()))
            os.environ["E2E_PRICE_IN_PER_MTOK"] = "1"
            os.environ["E2E_PRICE_OUT_PER_MTOK"] = "2"
            plug.stop()
            raw = open(os.path.join(tmp, "cost.json")).read()
            cost = json.loads(raw)
            v.check("cost.json has the model, the requests and the tokens from the replies' usage, priced when "
                    "both prices are set",
                    cost["model"] == "m-1" and cost["requests"] == 7 and cost["input_tokens"] == 200
                    and cost["output_tokens"] == 40 and abs(cost["usd_estimate"] - 0.00028) < 1e-9, raw)
            v.check("the key is never in cost.json or on a file of the run", key not in raw
                    and app_mod.files_holding(tmp, key, skip=()) == [], str(app_mod.files_holding(tmp, key, skip=())))
        with vllm_env(**base):
            keyless = vllm_mod.make(tmp, {"run_dir": tmp})
            v.check("without VLLM_API_KEY the plug is keyless: no key variable, no credentials on the records",
                    keyless.key_env is None and not keyless.keyed)
            keyless.stop()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_gpu(v: Verdict) -> None:
    """Run the launcher as a child with no model variable (`child_env`), with every one of them set
    here (fakes; BACKEND=anthropic): the gpu tests would run on the vllm plug and skip, exit 0, and
    `--only-gpu --dry-run` collects their probes and starts nothing."""
    planted = {"BACKEND": "anthropic", "E2E_MODEL": "claude-selftest", "E2E_WORKER_MODEL": "claude-selftest-worker", "ANTHROPIC_CACHING": "none",
               "E2E_ANTHROPIC_KEY": "selftest-" + secrets.token_hex(8),
               "ANTHROPIC_API_KEY": "selftest-" + secrets.token_hex(8),
               "VLLM_ENDPOINT": "https://gpu.example.invalid/v1", "FANTASTIC_AI_MODEL": "m-1",
               "VLLM_KV_SLOTS": "2", "VLLM_KV_OVERFLOW": "evict", "VLLM_API_KEY": "door-" + secrets.token_hex(8),
               "MAX_TOKENS": "16", "E2E_MAX_REQUESTS": "1", "E2E_PRICE_IN_PER_MTOK": "1",
               "E2E_PRICE_OUT_PER_MTOK": "2",
               "E2E_UPSTREAM": "http://127.0.0.1:9"}  # discard: had a child kept a key, nothing would answer
    with model_env(**planted):
        v.check("every model variable is planted for this check", set(MODEL_ENV) <= set(planted))
        env = child_env(os.environ)
    me = os.path.abspath(__file__)
    r = subprocess.run([sys.executable, me, "--only-gpu", "--dry-run"], env=env, capture_output=True,
                       text=True, timeout=120)
    stripped = all(f"COLLECT {n}  (costs: gpu, plug: vllm)  (would SKIP: not set: VLLM_ENDPOINT" in r.stdout
                   for n in GPU_TESTS)
    v.check("the child carries no model variable: with them all set here, its gpu tests are on the vllm "
            "plug and would SKIP", r.returncode == 0 and stripped, (r.stdout + r.stderr)[-800:])
    v.check("the gpu tests collect under --only-gpu --dry-run without the variables",
            r.returncode == 0 and f"{len(GPU_TESTS)}/{len(GPU_TESTS)} probes collect" in r.stdout,
            (r.stdout + r.stderr)[-800:])
    if stripped:  # else the child would run the real tests (and its selftest a child of its own)
        r = subprocess.run([sys.executable, me, "--only-gpu"], env=env, capture_output=True, text=True,
                           timeout=120)
        skips = all(f"SKIP {n} " in r.stdout for n in GPU_TESTS)
        out = (r.stdout + r.stderr)[-800:]
    else:
        skips, out = False, "not started: the child's environment holds a model variable"
    v.check("the gpu tests skip without the variables and the run exits 0", skips and r.returncode == 0, out)
    by = {t["name"]: t for t in discover()}
    bad = []
    for n in GPU_TESTS:
        t = by.get(n)
        twin = by.get(n.replace("gpu_cli_ssh/", "cli_ssh/").replace("gpu_cli/", "cli/"))
        files = t and all(os.path.isfile(os.path.join(t["dir"], f)) for f in ("README.md", "test.yaml", "probe.py"))
        if not (t and twin and files and t["costs"] == "gpu" and t["declared_plug"] == "vllm"
                and not os.path.exists(os.path.join(t["dir"], "script.json"))):
            bad.append(n)
    v.check("each gpu test has a cli twin, its README, test.yaml and probe, costs gpu on the vllm plug, no script.json",
            not bad, str(bad))


def selftest_backend(v: Verdict) -> None:
    """BACKEND picks the plug of the gpu tests only; `--only-gpu --dry-run` collects all ten with either."""
    gpu, free = {"costs": "gpu", "plug": "vllm"}, {"costs": "free"}
    turns = {"costs": "turns", "plug": "anthropic"}
    with model_env():
        v.check("without BACKEND each test takes its test.yaml's plug",
                (plug_of(gpu), plug_of(free), plug_of(turns)) == ("vllm", "scripted", "anthropic"))
    with model_env(BACKEND="anthropic"):
        v.check("BACKEND=anthropic: a gpu test takes the anthropic plug, a free or turns test keeps its own",
                (plug_of(gpu), plug_of(free), plug_of(turns)) == ("anthropic", "scripted", "anthropic"))
        by = {t["name"]: t for t in discover()}
        moved = sorted(n for n, t in by.items() if t["plug"] != t["declared_plug"])
        v.check("discover: the ten gpu tests (and only they) are on the anthropic plug, declared on vllm",
                moved == sorted(GPU_TESTS) and all(by[n]["plug"] == "anthropic" and by[n]["declared_plug"] == "vllm"
                                                   for n in GPU_TESTS), str(moved))
    with model_env(BACKEND="vllm"):
        v.check("BACKEND=vllm: a gpu test declared on another plug takes vllm; a free test keeps scripted",
                plug_of({"costs": "gpu", "plug": "anthropic"}) == "vllm" and plug_of(free) == "scripted")
    with model_env(BACKEND="openai"):
        try:
            plug_of(gpu)
            refused = ""
        except SystemExit as e:
            refused = str(e)
        v.check("a BACKEND that is neither vllm nor anthropic is refused (a free test never reads it)",
                "BACKEND must be vllm or anthropic" in refused and plug_of(free) == "scripted", refused)
    me = os.path.abspath(__file__)
    for backend, why in (("vllm", "not set: VLLM_ENDPOINT"), ("anthropic", "no key in the environment")):
        env = dict(child_env(os.environ), BACKEND=backend)
        r = subprocess.run([sys.executable, me, "--only-gpu", "--dry-run"], env=env, capture_output=True,
                           text=True, timeout=120)
        ok = all(f"COLLECT {n}  (costs: gpu, plug: {backend})  (would SKIP: {why}" in r.stdout for n in GPU_TESTS)
        v.check(f"BACKEND={backend}: --only-gpu --dry-run collects the ten on that plug, each would SKIP here",
                r.returncode == 0 and ok and f"{len(GPU_TESTS)}/{len(GPU_TESTS)} probes collect" in r.stdout,
                (r.stdout + r.stderr)[-800:])


def selftest_selection(v: Verdict) -> None:
    allt = discover()
    free = pick(allt, only_free=True)
    turns = pick(allt, only_turns=True)
    v.check("--only-free never selects a turns test",
            free and not any(t["costs"] == "turns" for t in free))
    v.check("--only-turns selects the turns tests and only them",
            {t["name"] for t in turns} == set(TURNS_TESTS), str([t["name"] for t in turns]))
    gpu = pick(allt, only_gpu=True)
    v.check("--only-free never selects a gpu test; --only-gpu selects the gpu tests and only them",
            not any(t["costs"] == "gpu" for t in free) and {t["name"] for t in gpu} == set(GPU_TESTS)
            and not any(t["costs"] == "gpu" for t in turns), str([t["name"] for t in gpu]))
    v.check("--only-turns never selects a gpu test",
            not any(t["costs"] == "gpu" for t in turns))
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
             "app/attach_remote")


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
        test = {"name": "g/skipper", "dir": tdir, "costs": "free", "timeout": 30, "plug": "scripted"}
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
    """Team.start runs fantastic's own words in the run folder: `--headless` unless told, no `--dir`."""
    tmp = tempfile.mkdtemp(prefix="e2e-args-")
    t = None
    try:
        stub = os.path.join(tmp, "stub")
        with open(stub, "w") as f:
            f.write("#!/bin/sh\nprintf '%s\\n' \"$@\" > argv.txt\npwd > cwd.txt\nmkdir -p .fantastic/cli\n"
                    ": > .fantastic/cli/mcp.sock\nexec sleep 20\n")
        os.chmod(stub, 0o755)
        t = team_mod.Team(stub, tmp, {})
        t.start(wait_s=10, extra_args=["webui", "--port", 18080])
        argv = open(os.path.join(tmp, "argv.txt")).read().split("\n")[:-1]
        v.check("Team.start passes fantastic's words as they are", argv == ["webui", "--port", "18080"], str(argv))
        v.check("Team.port is the port of the start", t.port == 18080)
        v.check("it runs in the run folder (fantastic has no --dir)",
                os.path.realpath(open(os.path.join(tmp, "cwd.txt")).read().strip()) == os.path.realpath(tmp))
        t.stop()
        os.remove(os.path.join(tmp, ".fantastic", "cli", "mcp.sock"))
        t.start(wait_s=10)
        argv = open(os.path.join(tmp, "argv.txt")).read().split("\n")[:-1]
        v.check("Team.start with no extra_args is [--headless] only, and clears the port",
                argv == ["--headless"] and t.port is None, str(argv))
        v.check("Team.argv builds the same command line",
                t.argv(["webui", "--port", 7]) == [stub, "webui", "--port", "7"] and "--dir" not in t.argv())
    finally:
        if t:
            t.stop()
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_cli_fixture(v: Verdict) -> None:
    """The cli group's fixture: unsolved at first, an edit alone is not solved, a commit is."""
    tmp = tempfile.mkdtemp(prefix="e2e-fixture-")
    try:
        repo = cli_mod.make_fixture(tmp)
        v.check("the fixture's test fails", not cli_mod.test_passes(repo))
        v.check("the fixture is not solved", not cli_mod.solved(repo)[0])
        for f in (("adder.py", "a - b", "a + b"), ("muler.py", "a + b", "a * b")):
            p = os.path.join(repo, f[0])
            with open(p) as fh:
                text = fh.read().replace(f[1], f[2])
            with open(p, "w") as fh:
                fh.write(text)
        v.check("the fix in the work tree, uncommitted, is not solved", not cli_mod.solved(repo)[0])
        cli_mod.git(repo, "commit", "-q", "-a", "-m", "fix")
        v.check("the fix committed is solved", cli_mod.solved(repo)[0], cli_mod.solved(repo)[1])
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_scripted_lanes(v: Verdict) -> None:
    """Lanes: each agent (by the marker in its first message) plays its own turns in order, and
    `after` holds a turn until the named lanes have played out."""
    script = [{"text": "main-1"}, {"text": "a-1", "lane": "a"}, {"text": "a-2", "lane": "a"},
              {"text": "b-1", "lane": "b", "after": ["a"]}]
    m = scripted.ScriptedModel(script).start()
    try:
        def ask(marker):
            body = json.dumps({"messages": [{"role": "user", "content": f"go {marker}"}]}).encode()
            req = urllib.request.Request(m.url + "/v1/messages", data=body, method="POST")
            with urllib.request.urlopen(req, timeout=20) as r:
                return scripted.parse_stream(r.read())

        got = {}
        tb = threading.Thread(target=lambda: got.setdefault("b", ask("[lane:b]")))
        tb.start()
        time.sleep(0.5)
        v.check("a turn with `after` is held while the lane it waits for has turns left", "b" not in got)
        outs = [ask("[lane:a]"), ask("[lane:a]")]
        tb.join(20)
        v.check("lane a plays a-1, a-2 in order", outs == ["a-1", "a-2"], str(outs))
        v.check("the held turn is answered once lane a is played out", got.get("b") == "b-1", str(got))
        v.check("a request with no marker is the main lane", ask("plain") == "main-1")
        v.check("every turn played, none past the end", m.consumed and m.request_lanes[-1] == "main",
                str(m.request_lanes))
        try:
            ask("[lane:a]")
        except urllib.error.HTTPError:
            pass
        v.check("{{id:N}} is the Nth <id> in the request body",
                m.resolve_ids("x {{id:2}} {{id:3}}", "<id>a1</id> <id>b2</id>") == "x b2 {{id:3}}")
        v.check("a lane asked past its end is an overrun, not another lane's turn", m.overrun == 1)
    finally:
        m.stop()


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
    v.check("the four #37 e2e tests exist", set(NEW_TESTS) <= set(by), str(sorted(set(NEW_TESTS) - set(by))))
    bad = []
    for n in NEW_TESTS:
        t = by.get(n)
        if not t:
            continue
        ok, detail = collect_probe(t["dir"])
        files = all(os.path.isfile(os.path.join(t["dir"], f)) for f in ("README.md", "test.yaml", "script.json", "probe.py"))
        if not (ok and files and t["costs"] == "free" and t["plug"] == "scripted"):
            bad.append(f"{n}: collect={detail} files={files} costs={t['costs']} plug={t['plug']}")
    v.check("each collects, has its files, and is free / scripted", not bad, "; ".join(bad))


class podman_env(vllm_env):
    """The Podman remote helper's variables for a block (`E2E_PODMAN`, the build dirs): set the given
    ones, clear the rest, restore after."""
    names = ("E2E_PODMAN", "E2E_PODMAN_BUILD_DIR", "CARGO_TARGET_DIR", "FAKE_PODMAN_LOG", "FAKE_PODMAN_MODE")


# A fake `podman`: logs each argv (and whether AUTHORIZED_KEY came in its environment) and answers
# as FAKE_PODMAN_MODE says: `down` (no machine), `noport` (the container maps no port), `up`.
FAKE_PODMAN = """#!/bin/sh
printf '%s|key=%s\\n' "$*" "${AUTHORIZED_KEY:+set}" >> "$FAKE_PODMAN_LOG"
case "$FAKE_PODMAN_MODE:$1" in
  down:info) echo "Cannot connect to Podman. Is the machine running?" >&2; exit 125 ;;
  pasta:info) echo "false true pasta" ;;
  *:info) echo arm64 ;;
  *:image) exit 0 ;;
  *:run) echo c0ffee ;;
  noport:port) exit 125 ;;
  *:port) echo 127.0.0.1:9 ;;
  *:container) exit 1 ;;
esac
exit 0
"""


def selftest_podman_remote(v: Verdict) -> None:
    """The ssh remote's helper (#242) without Podman: SKIP without it or its machine, the ssh command
    lines, the gateway URL and its network options, the source tag, the image files, the cache by tag, and teardown on a
    failed start."""
    from helpers.verdict import Skip
    pr = podman_remote
    tmp = tempfile.mkdtemp(prefix="e2e-podman-")
    try:
        with podman_env(E2E_PODMAN=os.path.join(tmp, "no-such-podman")):
            v.check("no podman: why_not says so", pr.why_not() == "no podman on this machine", str(pr.why_not()))
            try:
                pr.require()
                skipped = None
            except Skip as e:
                skipped = e.reason
            v.check("no podman: require() raises Skip with the reason", skipped == "no podman on this machine",
                    str(skipped))
        fake = os.path.join(tmp, "podman")
        with open(fake, "w") as f:
            f.write(FAKE_PODMAN)
        os.chmod(fake, 0o755)
        log = os.path.join(tmp, "podman.log")
        with podman_env(E2E_PODMAN=fake, FAKE_PODMAN_LOG=log, FAKE_PODMAN_MODE="down"):
            why = pr.why_not() or ""
            v.check("a podman with no running machine: why_not names it",
                    why.startswith("podman has no running machine") and "machine running" in why, why)
        with podman_env(E2E_PODMAN=fake, FAKE_PODMAN_LOG=log, FAKE_PODMAN_MODE="up"):
            v.check("podman with a running machine: nothing stops it", pr.why_not() is None, str(pr.why_not()))

        argv = pr.ssh_argv(2222, "/k/id", forward=(4100, 8765))
        v.check("the tunnel is ssh -N -L local:127.0.0.1:remote on the run's key only, no prompt",
                argv[0] == "ssh" and "-N" in argv and "4100:127.0.0.1:8765" in argv
                and argv[argv.index("-p") + 1] == "2222" and argv[argv.index("-i") + 1] == "/k/id"
                and {"BatchMode=yes", "IdentitiesOnly=yes", "ExitOnForwardFailure=yes"} <= set(argv)
                and argv[-1] == "team@127.0.0.1", str(argv))
        argv = pr.ssh_argv(2222, "/k/id", "cd w && fantastic_team send '{}'", forward=(4100, 8765))
        v.check("a raw command with a tunnel ends with the command and has no -N",
                argv[-1] == "cd w && fantastic_team send '{}'" and "-N" not in argv and argv[-2] == "team@127.0.0.1")
        v.check("the gateway URL: 127.0.0.1 and localhost become host.containers.internal, others stay",
                pr.gateway_url("http://x:t@127.0.0.1:5/a.git") == "http://x:t@host.containers.internal:5/a.git"
                and pr.gateway_url("http://localhost:7/") == "http://host.containers.internal:7/"
                and pr.gateway_url("https://example.invalid:8/") == "https://example.invalid:8/")

        src = os.path.join(tmp, "src")
        os.makedirs(os.path.join(src, "e2e"))
        for name, text in (("rust-toolchain.toml", '[toolchain]\nchannel = "1.2.3"\n'), ("a.rs", "1"),
                           ("e2e/t.py", "1")):
            with open(os.path.join(src, name), "w") as f:
                f.write(text)
        cli_mod.git(src, "init", "-q", "-b", "main")
        cli_mod.git(src, "add", "-A")
        cli_mod.git(src, "commit", "-q", "-m", "src")
        first = pr.source_tag(src)
        v.check("the source tag is stable for one tree", pr.source_tag(src) == first)
        with open(os.path.join(src, "e2e", "t.py"), "w") as f:
            f.write("2")
        v.check("an edit under e2e/ keeps the tag (the binary is the same)", pr.source_tag(src) == first)
        with open(os.path.join(src, "a.rs"), "w") as f:
            f.write("2")
        edited = pr.source_tag(src)
        v.check("an uncommitted edit of the source changes the tag", edited != first)
        with open(os.path.join(src, "b.rs"), "w") as f:
            f.write("new")
        v.check("an untracked source file changes the tag", pr.source_tag(src) not in (first, edited))
        v.check("the toolchain is rust-toolchain.toml's channel", pr.toolchain(src) == "1.2.3")

        with podman_env(E2E_PODMAN_BUILD_DIR=os.path.join(tmp, "b")):
            v.check("E2E_PODMAN_BUILD_DIR is the build dir", pr.build_dir() == os.path.join(tmp, "b"))
        with podman_env(CARGO_TARGET_DIR=os.path.join(tmp, "t")):
            v.check("else <CARGO_TARGET_DIR>/podman-linux", pr.build_dir() == os.path.join(tmp, "t", "podman-linux"))
        b = pr.builder_argv("/repo", "/build", "1.2.3")
        v.check("the builder mounts the source read-only and the target and registry from the build dir",
                "/repo:/src:ro" in b and "/build:/build" in b and "CARGO_HOME=/build/cargo-home" in b
                and "CARGO_TARGET_DIR=/build/target" in b and "--locked" in b
                and f"{pr.BUILDER}:{pr.builder_tag('1.2.3')}" in b and pr.builder_tag("1.2.3").startswith("1.2.3-"), str(b))
        box = (pr.IMAGE_FILE + pr.SSHD_CONFIG + pr.ENTRYPOINT).replace("{user}", pr.USER)
        v.check("the box: sshd, git, python3, the binary; key login only, no root, no password",
                all(w in box for w in ("openssh-server", "git", "python3", "/usr/local/bin/fantastic_team",
                                       "PasswordAuthentication no", "PermitRootLogin no", "AllowUsers team",
                                       "authorized_keys", "sshd -D")))

        with podman_env(E2E_PODMAN=fake, FAKE_PODMAN_LOG=log, FAKE_PODMAN_MODE="up",
                        E2E_PODMAN_BUILD_DIR=os.path.join(tmp, "b")):
            open(log, "w").close()
            ref = pr.image(src, log=lambda *_: None)
            calls = open(log).read().splitlines()
            v.check("an image whose tag exists is used as it is: nothing is built",
                    ref == f"{pr.IMAGE}:{pr.source_tag(src)}" and calls
                    and all(c.startswith("image exists") for c in calls), str(calls))

        run_dir = os.path.join(tmp, "run")
        os.makedirs(run_dir)
        with podman_env(E2E_PODMAN=fake, FAKE_PODMAN_LOG=log, FAKE_PODMAN_MODE="noport"):
            open(log, "w").close()
            r = pr.Remote(run_dir, "img:tag", log=lambda *_: None)
            try:
                with r:
                    pass
                failed = False
            except pr.RemoteError:
                failed = True
            calls = open(log).read().splitlines()
            runs = [c for c in calls if c.startswith("run ")]
            pub = open(os.path.join(run_dir, "remote-key", "id_remote.pub")).read().split()[1]
            v.check("a start that fails raises, and the container is removed anyway",
                    failed and f"rm -f -t 5 {r.name}|key=" in calls and not r.started, str(calls))
            v.check("the container maps sshd on 127.0.0.1 only and takes the key from the environment, not the argv",
                    len(runs) == 1 and "-p 127.0.0.1::22" in runs[0] and runs[0].endswith("|key=set")
                    and pub not in open(log).read(), str(runs))
            key = os.path.join(run_dir, "remote-key", "id_remote")
            v.check("a throwaway key per run in the run folder, 0600",
                    os.path.isfile(key) and (os.stat(key).st_mode & 0o777) == 0o600)
            open(log, "w").close()
            r.stop()
            v.check("stop after teardown removes nothing twice", open(log).read() == "")
            runs = [c for c in calls if c.startswith("run ")]
            v.check("a podman machine (or no pasta): the container takes no network options",
                    pr.network_args() == [] and "--network" not in runs[0] and "--add-host" not in runs[0], str(runs))
        with podman_env(E2E_PODMAN=fake, FAKE_PODMAN_LOG=log, FAKE_PODMAN_MODE="pasta"):
            open(log, "w").close()
            pr.Remote(run_dir, "img:tag", log=lambda *_: None)._run("img:tag", "ssh-ed25519 AAAA e2e")
            runs = [c for c in open(log).read().splitlines() if c.startswith("run ")]
            v.check("local rootless pasta: the host's loopback is mapped and the gateway name pinned to it",
                    len(runs) == 1 and f"--network pasta:--map-host-loopback,{pr.HOST_LOOPBACK}" in runs[0]
                    and f"--add-host {pr.GATEWAY}:{pr.HOST_LOOPBACK}" in runs[0], str(runs))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_ssh_twins(v: Verdict) -> None:
    """The ssh twins (#242) without Podman: the scripts match their builders, every placeholder is
    bound, the plug's late binding and minted ids, the role cards' quoting, the gpu twins' plain-words
    wiring, and the fixture published to a git server the local clone pushes to with no token in
    a command."""
    T = ssh_twins
    v.check("each cli_ssh script.json is what helpers/ssh_twins.py writes (python3 -m helpers.ssh_twins --write)",
            T.stale_scripts() == [], str(T.stale_scripts()))
    by = {t["name"]: t for t in discover()}
    bad = []
    for n in SSH_TESTS:
        t = by.get(n)
        ok = t and collect_probe(t["dir"])[0] and t["costs"] == "free" and t["plug"] == "scripted" and all(
            os.path.isfile(os.path.join(t["dir"], f)) for f in ("README.md", "test.yaml", "script.json", "probe.py"))
        if not ok:
            bad.append(n)
    v.check("the five cli_ssh twins exist, collect, and are free on the scripted plug", not bad, str(bad))
    values = {"ssh_port": "2200", "ssh_key": "/k/id", "tunnel_port": "4100",
              "master_guid": "0123456789abcdef", "remote_git_url": "http://host.containers.internal:8/acme/widgets.git"}
    left = []
    for n in CLI_SCENARIOS:
        m = scripted.ScriptedModel(T.SCRIPTS[n]())
        try:
            m.bind(**values)
        except ValueError as e:
            left.append(f"{n}: {e}")
    v.check("every placeholder of the five scripts is one the box binds", not left, str(left))
    m = scripted.ScriptedModel([{"text": "{{a}} {{b}}"}], {"a": "1"})
    try:
        m.bind()
        unbound = False
    except ValueError:
        unbound = True
    v.check("a placeholder left open at load stays as written, and bind refuses one still open",
            m.turns == ["1 {{b}}"] and unbound, str(m.turns))
    v.check("bind fills it", m.bind(b="2").turns == ["1 2"], str(m.turns))
    v.check("{{minted:N}} is the Nth distinct minted id, other <id> tags skipped",
            m.resolve_ids("{{minted:1}} {{minted:2}}",
                          "<id>w2</id><id>anthropic_backend_0a1b2c</id><id>roles</id><id>anthropic_backend_0a1b2c</id>"
                          "<id>129</id><id>anthropic_backend_ffee00</id>") == "anthropic_backend_0a1b2c anthropic_backend_ffee00")
    v.check("the remote kernel starts in the tunnel's ssh session, its pid kept",
            T.kernel_start_command() == "mkdir -p w && cd w && echo $$ > team.pid && "
            "exec fantastic_team --headless >> team.log 2>&1")
    from helpers import gpu_cli as gpu_mod

    class History:
        """A lead whose last row is an assistant `done` (the grep finds it)."""
        rows = [{"id": 1, "kind": "user"}, {"id": 2, "kind": "assistant"}]

        def send(self, _id, payload, timeout=60):
            if payload["type"] == "history":
                return {"result": {"rows": list(self.rows), "next": None}}
            return {"result": {"total": 1}}
    lead = History()
    watch = gpu_mod.Lead(lead, "lead")
    watch.mark()
    early, again = watch.said_done(), watch.said_done()
    lead.rows = lead.rows + [{"id": 3, "kind": "user"}, {"id": 4, "kind": "assistant"}]
    v.check("a lead told twice: its first `done` does not count, polled again or not; a `done` after the new "
            "message does (gpu_ssh.play)", not early and not again and watch.said_done(), f"early {early} again {again}")
    v.check("a history re-read holds each row once", len(watch.rows) == 4, str(watch.rows))
    tmp = tempfile.mkdtemp(prefix="e2e-ssh-twins-")
    forge = None
    try:
        bare = forge_mod.make_bare(os.path.join(tmp, "forge"))
        forge_mod.seed_clone(bare, os.path.join(tmp, "seed"))
        forge = forge_mod.Forge(os.path.join(tmp, "forge")).start()

        class Plug:
            keyed = False
            endpoint = "http://127.0.0.1:9/v1"

            def ai_record(self):
                return {"handler_module": "vllm_backend.tools", "model": "m", "endpoint": self.endpoint}

        class Guid:
            def send(self, target, payload, timeout=60):
                return {"result": {"guid": "0123456789abcdef"}} if payload["type"] == "reflect" else {}

        class Ctx:
            pass
        ctx = Ctx()
        ctx.run_dir = ctx.work = tmp
        ctx.forge, ctx.model, ctx.team = forge, Plug(), Guid()
        os.makedirs(os.path.join(tmp, "sandbox"))
        ctx.repo = cli_mod.make_fixture(tmp)
        box = T.Box(ctx)
        box.publish()
        top = subprocess.run(["git", "-C", bare, "log", "--format=%s", "main"], capture_output=True, text=True).stdout
        v.check("publish: the fixture is the git server's main", top.split("\n")[0] == "calc with two bugs", top)
        with open(os.path.join(ctx.repo, "adder.py"), "a") as f:
            f.write("# pushed\n")
        env = dict(os.environ, HOME=os.path.join(tmp, "sandbox"), GIT_CONFIG_NOSYSTEM="1", GIT_TERMINAL_PROMPT="0")
        git = lambda *a: subprocess.run(["git", "-c", "user.name=W", "-c", "user.email=w@example.invalid", *a],
                                        cwd=ctx.repo, env=env, capture_output=True, text=True)
        git("commit", "-q", "-a", "-m", "pushed")
        r = git("push", "-q", "origin", "HEAD:main")
        top = subprocess.run(["git", "-C", bare, "log", "--format=%s", "-1", "main"], capture_output=True, text=True).stdout
        v.check("the local clone pushes over HTTP with the sandbox's git config (no token in the command)",
                r.returncode == 0 and top.strip() == "pushed" and forge.token not in git("remote", "-v").stdout,
                r.stderr[-300:])
        box.remote.port, box.remote.key = 2200, "/k/id"
        text = gpu_ssh.wiring(ctx, box, {"worker.md": "card"})
        v.check("the gpu twins' plain words name the box (port, key, login), the tunnel and the git server via the gateway",
                all(x in text for x in ("2200", "/k/id", "team@127.0.0.1", f"{box.tunnel_port}:127.0.0.1:{box.tunnel_port}",
                                         "host.containers.internal", T.BRIDGE, T.LEG, "worker.md")), text[:300])
        v.check("and never the git server's token", forge.token not in text)
        # the git twin's box: no token on it and none in the master's sandbox, the git server dialled on the box's own loopback
        sub = os.path.join(tmp, "git_twin")
        os.makedirs(os.path.join(sub, "sandbox"))
        ctx2 = Ctx()
        ctx2.run_dir, ctx2.work = tmp, sub
        ctx2.forge, ctx2.model, ctx2.team = forge, Plug(), Guid()
        ctx2.repo = cli_mod.make_fixture(sub)
        bare_box = T.Box(ctx2, git_token=False)
        bare_box.publish()
        top = subprocess.run(["git", "-C", bare, "log", "--format=%s", "-1", "main"], capture_output=True, text=True).stdout
        v.check("publish without a token: the fixture is on the git server, the master's sandbox has no git config",
                top.strip() == "calc with two bugs" and not os.path.exists(os.path.join(sub, "sandbox", ".gitconfig")), top)
        bare_box.remote.port, bare_box.remote.key = 2200, "/k/id"
        have, gate = bare_box.values(), box.values()
        v.check("without a token the box dials the git server on its own 127.0.0.1 (the relay's port is the server's), with one "
                "through Podman's gateway; the forge's port is bound only for the former",
                have["remote_git_url"] == T.git_url(ctx2) and have["forge_port"] == str(forge.port)
                and "host.containers.internal" in gate["remote_git_url"] and "forge_port" not in gate, f"{have} {gate}")
    finally:
        if forge:
            forge.stop()
        shutil.rmtree(tmp, ignore_errors=True)


def selftest_remote_work(v: Verdict) -> None:
    """The remote half of the ssh twins (#252) without Podman: the box holds no backend, the work
    program (run against a stub of fantastic's connector) and the checks on what it saw, each with the
    mutations they must refuse, the plug's `grant_leg` turns, and the leg's grants."""
    T = ssh_twins
    # no script and no setup puts a backend on the box
    made = []
    bad_setup = "backend" in T.setup_command() or "endpoint" in T.setup_command()
    for n in T.SCENARIOS:
        for turn in T.SCRIPTS[n]():
            for c in turn["calls"]:
                p = ((c.get("args") or {}).get("payload") or {}).get("payload") or {}
                if p.get("type") == "create_agent":
                    made.append(p.get("handler_module"))
    v.check("the box is given no backend: no script or setup command makes an AI agent or names a model endpoint there",
            not bad_setup and not made, f"{bad_setup} {sorted(set(made))}")
    v.check("the remote exec agent may call the master's agents send/watch/status/stop only",
            T.REMOTE_EXEC["can_call"] == {"$self": "*", "{{master_guid}}|*": ["send", "watch", "status", "stop"]})
    v.check("the bridge record grants its leg exactly those verbs on the backends named, and nothing without",
            T.bridge_record(["w2"])["can_call"] == {"$self": "*", "w2": T.BACKEND_VERBS}
            and "can_call" not in T.bridge_record())
    # the arrival check, and its mutations
    row = lambda type, phase=None, client="w2", **k: {"type": type, **({"phase": phase} if phase else {}), "client_id": client, **k}
    good = [row("status", "thinking"), row("status", "streaming"), row("part_start", kind="text"), row("part_delta"),
            row("part_end"), row("status", "done")]
    ok, why = T.streamed(good, "w2")
    v.check("streamed: status and part_* before done is a stream that crossed", ok, why)
    mutations = {
        "unwatched (nothing arrived)": [],
        "a refused watch (an error event)": [{"type": None, "error": "unauthorized", "reason": "unauthorized"}],
        "parts only after done": [row("status", "thinking"), row("status", "done"), row("part_start", kind="text")],
        "no part at all": [row("status", "thinking"), row("status", "done")],
        "no status before the parts": [row("part_start", kind="text"), row("status", "done")],
        "another client's stream": [dict(r, client_id="other") for r in good],
        "never done": good[:-1],
    }
    leaked = [k for k, rows in mutations.items() if T.streamed(rows, "w2")[0]]
    v.check("streamed refuses each mutation (unwatched, refused, parts after done, none, no status, other client, no done)",
            not leaked, str(leaked))
    # the work program, against a stub connector
    tmp = tempfile.mkdtemp(prefix="e2e-work-")
    try:
        def play(stub: str, *flags, label: str = "w2") -> tuple[int, dict, list[dict]]:
            sub = os.path.join(tmp, f"run{len(os.listdir(tmp))}", "repo")
            os.makedirs(sub)
            prog = os.path.join(sub, "..", "work.py")
            with open(prog, "w") as f:
                f.write(stub + T.WORK_PROGRAM)
            r = subprocess.run([sys.executable, "-I", prog, *flags, "GUID", "w2", label, "do", "the", "work"],
                               cwd=sub, capture_output=True, text=True, timeout=60)
            try:
                result = json.load(open(os.path.join(sub, "..", f"{label}.result.json")))
            except (OSError, ValueError):
                result = {"stderr": r.stderr[-300:]}
            try:
                rows = [json.loads(x) for x in open(os.path.join(sub, "..", f"{label}.events.jsonl")).read().splitlines()]
            except OSError:
                rows = []
            return r.returncode, result, rows

        events = json.dumps(good)
        stub = ("import json\nclass K:\n    def watch(self, src, cb):\n        self.cb = cb; self.src = src\n"
                "    def send(self, target, payload):\n        assert target == 'GUID|w2' and payload['client_id'] == 'w2'\n"
                f"        assert payload['text'] == 'do the work'\n        for p in json.loads({events!r}): self.cb(p)\n"
                "        return {'response': 'ok'}\nkernel = K()\n")
        code, result, rows = play(stub)
        v.check("the work program watches `<guid>|<backend>`, sends it the work as that client, notes the events in "
                "arrival order and exits 0", code == 0 and result.get("ok") and [r["type"] for r in rows] == [g["type"] for g in good]
                and T.streamed(rows, "w2")[0], f"{code} {result} {rows[:2]}")
        code, result, rows = play(stub, "--no-watch")
        v.check("mutation: unwatched, the program still sends but notes nothing, and the arrival check refuses it",
                rows == [] and not T.streamed(rows, "w2")[0], f"{code} {rows}")
        down = stub.replace("return {'response': 'ok'}", "return {'error': 'bridge_down: the link dropped', 'reason': 'transport_error'}")
        code, result, rows = play(down)
        v.check("a call in flight when the bridge drops: the reply is bridge_down, the program exits 1 and says so",
                code == 1 and "bridge_down" in (result.get("reply_error") or ""), f"{code} {result}")
        denied = stub.replace("self.cb = cb; self.src = src", "self.cb = cb; self.src = src; cb({'error': 'x may not call watch', 'reason': 'unauthorized'})")
        code, result, rows = play(denied)
        v.check("mutation: a sealed watch grant (the watch is refused as an event), the program exits 1",
                code == 1 and result.get("refused") and not T.streamed(rows, "w2")[0], f"{code} {result}")
        watch_only = ("import json\nclass K:\n    def watch(self, src, cb):\n"
                      f"        for p in json.loads({events!r}): cb(dict(p, client_id='master'))\n"
                      "    def send(self, target, payload):\n        assert payload == {'type': 'status'}\n"
                      "        return {'generating': False}\nkernel = K()\n")
        code, result, rows = play(watch_only, "--no-send", "--client", "master", label="b")
        ready = [p for p, _, fs in os.walk(tmp) if "b.ready" in fs]
        v.check("--no-send --client: the program only watches (a status call is its barrier, then `<label>.ready`), "
                "and ends on the done of that client's turn", code == 0 and result.get("done") == 1 and ready,
                f"{code} {result} {ready}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    # the plug's grant_leg turns
    got = []
    m = scripted.ScriptedModel([{"calls": [{"target": "a", "type": "x"}], "grant_leg": ["w2", "{{minted:1}}"]}], {})
    m.leg_grant = got.append
    m.play(json.dumps({"messages": [{"role": "user", "content": "<id>anthropic_backend_0a1b2c</id>"}]}))
    v.check("a turn with grant_leg opens its agents to the leg before it is answered, a minted id resolved from the request",
            got == [["w2", "anthropic_backend_0a1b2c"]], str(got))
    m = scripted.ScriptedModel([{"calls": [{"target": "a", "type": "x"}], "grant_leg": ["w2"]}], {})
    try:
        m.play("{}")
        loud = False
    except RuntimeError:
        loud = True
    v.check("a grant_leg turn with no leg_grant set is an error, not a silent skip", loud)
    # the leg's grants, on a stub kernel
    class Kernel:
        def __init__(self):
            self.rec = {"remote": {"can_call": {"$self": "*", "lead": ["x"]}}, "w2": {"callable_by": {"$self": "*", "lead": "*"}}}

        def agents(self):
            return {k: dict(v) for k, v in self.rec.items()}

        def send(self, target, payload, timeout=60):
            if payload["type"] == "list_agents":
                return {"status": "completed", "result": {"agents": [dict(v, id=k) for k, v in self.rec.items()]}}
            self.rec[payload["id"]].update({k: val for k, val in payload.items() if k not in ("type", "id")})
            return {"status": "completed", "result": {}}
    kernel = Kernel()
    kernel.agents = lambda: {k: dict(v, id=k) for k, v in kernel.rec.items()}
    T.grant_leg(kernel, ["w2"])
    v.check("grant_leg: both ends, exactly send/watch/status/stop, what each held kept",
            kernel.rec["remote"]["can_call"] == {"$self": "*", "lead": ["x"], "w2": T.BACKEND_VERBS}
            and kernel.rec["w2"]["callable_by"] == {"$self": "*", "lead": "*", "remote": T.BACKEND_VERBS}, str(kernel.rec))
    T.seal_watch(kernel, "w2")
    v.check("seal_watch: the backend no longer lets the leg watch (send, status, stop stay)",
            kernel.rec["w2"]["callable_by"]["remote"] == ["send", "status", "stop"], str(kernel.rec["w2"]))


def selftest_git_credentials(v: Verdict) -> None:
    """The git twin (#253) without Podman: the bridge's grant on the store is `get` and nothing else, the
    connector's record names the master's store and holds no value, and neither the script nor the box's
    setup writes a token."""
    T = ssh_twins
    v.check("the store's grant is `get` and nothing else; the bridge record grants it on the stores named, beside the backends'",
            T.STORE_VERBS == ["get"]
            and T.bridge_record(stores=[T.GIT_STORE])["can_call"] == {"$self": "*", T.GIT_STORE: ["get"]}
            and T.bridge_record(["w2"], [T.GIT_STORE])["can_call"] == {"$self": "*", "w2": T.BACKEND_VERBS, T.GIT_STORE: ["get"]}
            and "can_call" not in T.bridge_record())
    rec = T.remote_git_record()
    store = "{{master_guid}}|" + T.GIT_STORE
    v.check("the remote connector's record: `team_git`, its credentials the MASTER's store `<guid>|git_credentials` and a ref "
            "(nothing else), it may call that store's `get` only, and the record holds no value",
            rec["handler_module"] == "team_git.tools" and rec["credentials"] == {"store": store, "ref": T.GIT_REF}
            and rec["can_call"] == {"$self": "*", store: ["get"]} and "token" not in json.dumps(rec).lower().replace("x-access-token", ""),
            str(rec))
    script = json.dumps(T.script_git_credentials()) + T.git_box_command() + T.setup_command()
    v.check("neither the script nor the box's setup names a token, an Authorization header, a git config or a credential helper: the "
            "box is given none", not [w for w in ("Authorization", "extraHeader", ".gitconfig", "credential.helper", "x-access-token",
                                                   "E2E_GIT_TOKEN", "http.") if w in script], "")
    m = scripted.ScriptedModel(T.script_git_credentials())
    values = {"ssh_port": "2200", "ssh_key": "/k/id", "tunnel_port": "4100", "master_guid": "0123456789abcdef",
              "remote_git_url": "http://127.0.0.1:8/acme/widgets.git", "forge_port": "8"}
    try:
        m.bind(**values)
        left = ""
    except ValueError as e:
        left = str(e)
    v.check("every placeholder of the git twin's script is one the box binds", not left, left)
    v.check("the phases add up to the script: setup, work, close", sum(T.phases_git().values()) == len(T.script_git_credentials()),
            str(T.phases_git()))
    by = {t["name"]: t for t in discover()}
    t = by.get(GIT_TWIN)
    v.check("cli_ssh/git_credentials exists, collects, and is free on the scripted plug",
            bool(t) and collect_probe(t["dir"])[0] and t["costs"] == "free" and t["plug"] == "scripted" and all(
                os.path.isfile(os.path.join(t["dir"], f)) for f in ("README.md", "test.yaml", "script.json", "probe.py")))
    try:
        scripted.ScriptedModel(T.script_checkins_busy_stuck()).bind(**values)
        left = ""
    except ValueError as e:
        left = str(e)
    v.check("every placeholder of the busy/stuck twin's script is one the box binds", not left, left)
    for n in (BUSY_TWIN, BUSY_TWIN.replace("cli_ssh/", "cli/")):
        t = by.get(n)
        v.check(f"{n} exists, collects, and is free on the scripted plug",
                bool(t) and collect_probe(t["dir"])[0] and t["costs"] == "free" and t["plug"] == "scripted" and all(
                    os.path.isfile(os.path.join(t["dir"], f)) for f in ("README.md", "test.yaml", "script.json", "probe.py")))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    ap.add_argument("--only-free", action="store_true", help="run only tests whose costs is free")
    ap.add_argument("--only-turns", action="store_true",
                    help="run only tests whose costs is turns (a real model; by hand, never CI)")
    ap.add_argument("--only-gpu", action="store_true",
                    help="run only tests whose costs is gpu (a vLLM server; by hand, never CI)")
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
    tests = pick(tests, a.only, a.only_free, a.only_turns, a.only_gpu)
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
            print(f"SKIP {t['name']} ({why_not(t['plug'])})")
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
