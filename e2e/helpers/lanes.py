"""Lanes (#325): run the selected scenarios in a pool of N workers.

`--lanes 1` (the default) is the old loop: in this process, in the selected order, output on the
console. With N > 1 each scenario runs in its own child process (`run.py --child NAME`, so its
SIGALRM timeout and its stdout are its own) with stdout and stderr in its own log file; scenarios
marked `serial: true` in test.yaml (a display, a shared GPU) run alone after the pool.
The scheduler is plain: it takes `run_one(test) -> "PASS" | "FAIL" | "SKIP"` and knows no scenario.
"""

from __future__ import annotations

import os
import subprocess
import sys
import threading
from concurrent.futures import ThreadPoolExecutor

CHILD_EXIT = {"PASS": 0, "FAIL": 1, "SKIP": 77}  # a child's exit code is its result
TAIL_LINES = 60


def is_serial(meta_value) -> bool:
    return str(meta_value).strip().lower() in ("true", "yes", "1")


def schedule(tests: list[dict], lanes: int, run_one, on_done=None) -> dict:
    """Run every test; returns {name: result} in the order of `tests`.

    lanes <= 1: one after another in the given order (serial or not): today's loop.
    lanes > 1: the non-serial tests in a pool of `lanes` workers, then the serial ones, one at a time.
    A `run_one` that raises is a FAIL of that test, never a lost one."""
    results: dict = {}
    guard = threading.Lock()

    def one(t: dict) -> None:
        try:
            r = run_one(t)
        except Exception as e:  # the scheduler never loses a scenario
            print(f"lane error in {t['name']}: {e!r}", file=sys.stderr)
            r = "FAIL"
        with guard:
            results[t["name"]] = r
        if on_done:
            on_done(t, r)

    if lanes <= 1:
        for t in tests:
            one(t)
    else:
        pool = [t for t in tests if not t.get("serial")]
        alone = [t for t in tests if t.get("serial")]
        with ThreadPoolExecutor(max_workers=lanes) as ex:
            list(ex.map(one, pool))
        for t in alone:
            one(t)
    return {t["name"]: results[t["name"]] for t in tests}


def log_path(stamp_dir: str, name: str) -> str:
    """A scenario's log: beside the run folders, never inside one (run_test creates its folder)."""
    return os.path.join(stamp_dir, "_logs", name.replace("/", "__") + ".log")


def child_argv(run_py: str, name: str, binary: str, stamp_dir: str) -> list[str]:
    return [sys.executable, run_py, "--child", name, "--bin", binary, "--stamp-dir", stamp_dir]


def run_child(run_py: str, test: dict, binary: str, stamp_dir: str) -> str:
    """One scenario in its own process, its output in its own log. Returns its result."""
    path = log_path(stamp_dir, test["name"])
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as log:
        code = subprocess.run(child_argv(run_py, test["name"], binary, stamp_dir), stdout=log,
                              stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL).returncode
    for result, c in CHILD_EXIT.items():
        if code == c:
            return result
    return "FAIL"  # a crash, a signal, any other code


def failing_tails(results: dict, stamp_dir: str, lines: int = TAIL_LINES) -> str:
    """The log tails of the failed scenarios, one block each, in the order of `results`."""
    out = []
    for name, r in results.items():
        if r != "FAIL":
            continue
        path = log_path(stamp_dir, name)
        try:
            with open(path, errors="replace") as f:
                tail = "".join(f.readlines()[-lines:])
        except OSError:
            tail = "(no log)\n"
        out.append(f"\n===== FAIL {name}: last {lines} lines of {path}\n{tail}")
    return "".join(out)


def prebuild_image(tests: list[dict], why_not, image, log=print) -> bool:
    """Before the pool: the ssh twins share one image tag, so it is built ONCE here, not by N
    children at the same time. `why_not()` is None when the remote can run; a build error is
    printed and left to each twin to report as it does today. Returns whether an image was asked for."""
    if not any(t["name"].split("/")[0].endswith("_ssh") for t in tests):
        return False
    if why_not() is not None:
        return False
    try:
        image(log=log)
    except Exception as e:
        log(f"  remote: image build failed before the pool: {e}")
    return True
