"""The `vllm` plug: a real model on a vLLM server, reached directly. The gpu tier, run by hand only.

The test's AI records use `vllm_backend.tools` and point straight at the server: there is no relay
between the team and the GPU. Everything the plug knows comes from the environment, the same
variables as fantastic's `vllm` plug (aisixteen/fantastic `e2e/backends/__init__.py`, `_vllm`):

  VLLM_ENDPOINT      the server's `/v1` URL (no host is guessed)              required
  FANTASTIC_AI_MODEL the served model's name                                   required
  VLLM_KV_SLOTS      how many conversations may be live on the endpoint       required, 1 or 2
  VLLM_KV_OVERFLOW   `evict` (rotate the LRU out) or `reject` (refuse the newcomer)   required
  VLLM_API_KEY       the door's key; optional. The credential store reads it from the team's
                     environment (`backend: env`); it is never written to disk or printed.
  MAX_TOKENS         one completion's cap (default THINK_BUDGET)
  E2E_MAX_REQUESTS   a hard cap on model requests per run (default 200); past it the run ends

If a required variable is missing the test is SKIP, as a turns test is without its key.

THE SHARED GPU. The server's slots are shared with production, so a run keeps at most
MAX_SLOTS conversations live: `VLLM_KV_SLOTS` above that is refused (SKIP, with the reason), and
the backend rotates or rejects past the slots the record names.

COST. With no relay, the plug asks the team: every vllm agent keeps its own counts (the `usage`
verb: requests, prompt_tokens, output_tokens, from the replies' `usage` chunks), so a poller sums
them while the run goes (a spawned agent that is deleted keeps the last counts the poller saw).
`cost.json` is written on stop: model, requests, tokens, never a key. Past E2E_MAX_REQUESTS the
poller stops the team, which ends the run.

The sampling knobs `no_repeat_ngram_size` and `repetition_penalty` come from `sampling.default.json`
beside this file (`sampling.json` overrides it), as in fantastic: the measured fix for the
chain-of-thought repetition loop. `thinking_token_budget` is left to the backend's own default.
"""

from __future__ import annotations

import json
import os
import threading
from urllib.parse import urlsplit

HERE = os.path.dirname(os.path.abspath(__file__))
#: One completion's output reserve, not the window: the server enforces `max_tokens + prompt <=
#: context`, so a cap near the window leaves no room for the prompt (fantastic's THINK_BUDGET).
THINK_BUDGET = 65_536
DEFAULT_CAP = 200
MAX_SLOTS = 2
OVERFLOWS = ("evict", "reject")
KEY_ENV = "VLLM_API_KEY"
REQUIRED = (
    ("VLLM_ENDPOINT", "the vLLM /v1 URL; no host is guessed"),
    ("FANTASTIC_AI_MODEL", "the served model's name"),
    ("VLLM_KV_SLOTS", "how many conversations may be live on the endpoint"),
    ("VLLM_KV_OVERFLOW", "evict (rotate the LRU out) or reject (refuse the newcomer)"),
)
POLL_S = 2.0  # seconds between two reads of the agents' usage


def problem() -> str | None:
    """Why the plug cannot run here, or None. Reads names and shapes, never the key."""
    missing = [f"{name} ({why})" for name, why in REQUIRED if not os.environ.get(name)]
    if missing:
        return "not set: " + ", ".join(m.split(" ", 1)[0] for m in missing)
    slots = os.environ["VLLM_KV_SLOTS"]
    if not slots.isdigit() or not 1 <= int(slots) <= MAX_SLOTS:
        return (f"VLLM_KV_SLOTS must be 1..{MAX_SLOTS} (the GPU's slots are shared with "
                f"production), got {slots!r}")
    if os.environ["VLLM_KV_OVERFLOW"] not in OVERFLOWS:
        return f"VLLM_KV_OVERFLOW must be one of {' | '.join(OVERFLOWS)}, got {os.environ['VLLM_KV_OVERFLOW']!r}"
    try:
        parts = urlsplit(os.environ["VLLM_ENDPOINT"])
    except ValueError as why:
        return f"VLLM_ENDPOINT does not parse ({why})"
    if parts.scheme not in ("http", "https") or not parts.netloc:
        return f"VLLM_ENDPOINT must be an http(s) URL, got {os.environ['VLLM_ENDPOINT']!r}"
    return None


def available() -> bool:
    return problem() is None


def runner_params() -> dict:
    """The sampling knobs beside this file: defaults, then a local `sampling.json`."""
    out: dict = {}
    for name in ("sampling.default.json", "sampling.json"):
        path = os.path.join(HERE, name)
        if not os.path.exists(path):
            continue
        try:
            with open(path) as f:
                data = json.load(f)
        except (ValueError, OSError):
            continue
        out.update({k: v for k, v in data.items() if not k.startswith("_")})
    return {k: v for k, v in out.items() if v is not None}


def _price(name: str) -> float | None:
    v = os.environ.get(name)
    return float(v) if v else None


class VllmPlug:
    real = True  # a real model answers: the launcher reads the key for the team, if there is one

    def __init__(self, variables: dict) -> None:
        self.run_dir = variables["run_dir"]
        self.endpoint = os.environ["VLLM_ENDPOINT"].rstrip("/")
        self.host = urlsplit(self.endpoint).netloc.lower()  # what the credential ref is bound to
        self.model_name = os.environ["FANTASTIC_AI_MODEL"]
        self.kv_slots = int(os.environ["VLLM_KV_SLOTS"])
        self.kv_overflow = os.environ["VLLM_KV_OVERFLOW"]
        self.max_tokens = int(os.environ.get("MAX_TOKENS") or THINK_BUDGET)
        self.key_env = KEY_ENV if os.environ.get(KEY_ENV) else None  # the name only
        self.keyed = self.key_env is not None
        self.cap = int(os.environ.get("E2E_MAX_REQUESTS", DEFAULT_CAP))
        self.counts: dict[str, dict] = {}  # agent id -> the last usage the poller saw
        self.polls = 0
        self.capped = False
        self.errored_reason: str | None = None
        self.lock = threading.Lock()
        self._team = None
        self._quit = threading.Event()
        self._thread: threading.Thread | None = None
        self._stopped = False
        self.poll_s = POLL_S

    # -- what the test's AI records say --------------------------------------------------
    def ai_record(self) -> dict:
        """The fields of an AI record that talks to this server (no credentials: the test adds
        them when the plug is keyed)."""
        rec = {"handler_module": "vllm_backend.tools", "model": self.model_name,
               "endpoint": self.endpoint, "max_tokens": self.max_tokens,
               "kv_slots": self.kv_slots, "kv_overflow": self.kv_overflow}
        runner = runner_params()
        for knob in ("no_repeat_ngram_size", "repetition_penalty"):
            if knob in runner:
                rec[knob] = runner[knob]
        return rec

    def recipe_args(self) -> list[str]:
        """fantastic's recipe that makes the root AI `ai` on this server: `ai vllm -p PORT --host H
        --kv-overflow O`. The recipe reads an http `/v1` URL; `recipe_fields` points it at the real one."""
        u = urlsplit(self.endpoint)
        port = u.port or (443 if u.scheme == "https" else 80)
        return ["--headless", "ai", "vllm", "-p", str(port), "--host", u.hostname or "",
                "--kv-overflow", self.kv_overflow]

    def recipe_fields(self) -> dict:
        """What the recipe leaves to `update_agent`: the server's `/v1` URL as given, the model,
        the budgets and the slots (the 2-slot rule), so nothing is read from the shared server."""
        return {k: v for k, v in self.ai_record().items() if k != "handler_module"}

    # -- the run's cost, asked of the team ------------------------------------------------
    def start(self) -> "VllmPlug":
        return self

    def watch(self, team) -> None:
        """Poll `team` for the vllm agents' usage until the plug stops. Called once the team is
        up (`helpers/cli.py start`)."""
        self._team = team  # a test that restarts the team (single_lead) hands over the new one
        if self._thread is None:
            self._thread = threading.Thread(target=self._loop, daemon=True)
            self._thread.start()

    def _loop(self) -> None:
        while not self._quit.wait(self.poll_s):
            self.poll()
            if self.requests > self.cap and not self.capped:
                self.capped = True
                self.errored_reason = f"request cap of {self.cap} reached (E2E_MAX_REQUESTS)"
                print(f"  vllm plug: {self.errored_reason}; stopping the team", flush=True)
                self._team.stop()
                return

    def poll(self) -> None:
        """Read the usage of every vllm agent now; a deleted agent keeps what was last seen."""
        team = self._team
        if team is None or not team.proc or team.proc.poll() is not None:
            return
        try:
            agents = team.agents()
        except Exception:  # the team is going down
            return
        for aid, rec in agents.items():
            if rec.get("handler_module") != "vllm_backend.tools":
                continue
            try:
                r = team.send(aid, {"type": "usage"})
            except Exception:
                continue
            u = r.get("result") if r.get("status") == "completed" else None
            if isinstance(u, dict) and "requests" in u:
                with self.lock:
                    old = self.counts.get(aid, {})
                    self.counts[aid] = {k: max(int(u.get(k) or 0), int(old.get(k) or 0))
                                        for k in ("requests", "prompt_tokens", "output_tokens")}
        self.polls += 1

    def finish(self, team) -> None:
        """One last read, before the team stops."""
        self._team = team
        self.poll()

    @property
    def requests(self) -> int:
        with self.lock:
            return sum(c["requests"] for c in self.counts.values())

    def totals(self) -> dict:
        """What the run cost. Never holds a key."""
        with self.lock:
            tin = sum(c["prompt_tokens"] for c in self.counts.values())
            tout = sum(c["output_tokens"] for c in self.counts.values())
            reqs = sum(c["requests"] for c in self.counts.values())
        pin, pout = _price("E2E_PRICE_IN_PER_MTOK"), _price("E2E_PRICE_OUT_PER_MTOK")
        usd = None
        if pin is not None and pout is not None:
            usd = round(tin * pin / 1e6 + tout * pout / 1e6, 8)
        return {"model": self.model_name, "requests": reqs, "replies": reqs,
                "errors": 1 if self.capped else 0, "input_tokens": tin, "output_tokens": tout,
                "usd_estimate": usd}

    def stop(self) -> None:
        if self._stopped:
            return
        self._stopped = True
        self._quit.set()
        if self._thread:
            self._thread.join(timeout=self.poll_s * 3 + 5)
        try:
            with open(os.path.join(self.run_dir, "cost.json"), "w") as f:
                json.dump(self.totals(), f, indent=2)
                f.write("\n")
        except OSError:
            pass

    @property
    def consumed(self) -> bool:
        return self.requests >= 1 and not self.capped

    @property
    def errored(self) -> bool:
        return self.capped


def make(test_dir: str, variables: dict) -> VllmPlug:
    why = problem()
    if why:
        raise RuntimeError(f"the vllm plug cannot run: {why}")
    return VllmPlug(variables)
