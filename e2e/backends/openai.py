"""The `openai` plug: a real model on an OpenAI-compatible server, reached directly. The gpu tier, run by hand only.

RAW MODE. The test's AI records use `openai_backend.tools` (aisixteen/fantastic#748):
`POST {endpoint}/chat/completions`, streamed; no tools array is sent and no `tool_calls` are
read, ai-core parses the model's `<call name=…>` text. Any server of that wire answers it
(llama.cpp's llama-server, …). There is no relay between the team and the server. Everything the
plug knows comes from the environment:

  OPENAI_ENDPOINT    the server's base URL up to `/v1` (no host is guessed)       required
  FANTASTIC_AI_MODEL the served model's name                                      required
  MAX_TOKENS         one completion's cap (default THINK_BUDGET)
  OPENAI_TEMPERATURE the chats' `temperature` knob (e.g. a model's recommended 1.0); unset = the backend's
  OPENAI_STOP        a JSON list of stop strings, the chats' `stop` knob (e.g. a model's native
                     tool token, which a raw-mode turn never needs); unset = none

If a required variable is missing the test is SKIP, as a turns test is without its key.

COST. The openai backend keeps no `usage` verb, so the plug cannot count requests through the
team: `cost.json` names the model, its counts are null, and there is no request cap (a local
server costs nothing per request).
"""

from __future__ import annotations

import json
import os
from urllib.parse import urlsplit

#: One completion's output reserve (fantastic's THINK_BUDGET), as the vllm plug's.
THINK_BUDGET = 65_536
REQUIRED = (
    ("OPENAI_ENDPOINT", "the server's base URL up to /v1 (e.g. http://127.0.0.1:8090/v1); no host is guessed"),
    ("FANTASTIC_AI_MODEL", "the served model's name"),
)


def problem() -> str | None:
    """Why the plug cannot run here, or None."""
    missing = [name for name, _ in REQUIRED if not os.environ.get(name)]
    if missing:
        return "not set: " + ", ".join(missing)
    stop = os.environ.get("OPENAI_STOP")
    if stop:
        try:
            ok = isinstance(json.loads(stop), list) and all(isinstance(x, str) for x in json.loads(stop))
        except ValueError:
            ok = False
        if not ok:
            return f"OPENAI_STOP must be a JSON list of strings, got {stop!r}"
    temp = os.environ.get("OPENAI_TEMPERATURE")
    if temp:
        try:
            float(temp)
        except ValueError:
            return f"OPENAI_TEMPERATURE must be a number, got {temp!r}"
    cap = os.environ.get("MAX_TOKENS")
    if cap is not None and (not cap.isdigit() or int(cap) < 1):
        return f"MAX_TOKENS must be a positive number of tokens, got {cap!r}"
    try:
        parts = urlsplit(os.environ["OPENAI_ENDPOINT"])
    except ValueError as why:
        return f"OPENAI_ENDPOINT does not parse ({why})"
    if parts.scheme not in ("http", "https") or not parts.netloc:
        return f"OPENAI_ENDPOINT must be an http(s) URL, got {os.environ['OPENAI_ENDPOINT']!r}"
    return None


def available() -> bool:
    return problem() is None


class OpenAiPlug:
    real = True  # a real model answers
    key_env = None  # a local server: no key, no credentials on the records
    keyed = False

    def __init__(self, variables: dict) -> None:
        self.run_dir = variables["run_dir"]
        self.endpoint = os.environ["OPENAI_ENDPOINT"].rstrip("/")
        self.host = urlsplit(self.endpoint).netloc.lower()
        self.model_name = os.environ["FANTASTIC_AI_MODEL"]
        self.max_tokens = int(os.environ.get("MAX_TOKENS") or THINK_BUDGET)
        self.stop_strings = json.loads(os.environ["OPENAI_STOP"]) if os.environ.get("OPENAI_STOP") else None
        self.temperature = float(os.environ["OPENAI_TEMPERATURE"]) if os.environ.get("OPENAI_TEMPERATURE") else None
        self._stopped = False

    def ai_record(self) -> dict:
        """The fields of an AI record that talks to this server; `max_tokens` is a chat's knob."""
        rec = {"handler_module": "openai_backend.tools", "model": self.model_name,
               "endpoint": self.endpoint, "max_tokens": self.max_tokens}
        if self.stop_strings:
            rec["stop"] = self.stop_strings
        if self.temperature is not None:
            rec["temperature"] = self.temperature
        return rec

    def recipe_args(self) -> list[str]:
        """fantastic's recipe that makes the root AI `ai` on this server: `ai openai --model M --endpoint URL`."""
        return ["--headless", "ai", "openai", "--model", self.model_name, "--endpoint", self.endpoint]

    def recipe_fields(self) -> dict:
        """What the recipe leaves to `update_agent`: the chat's knobs (output cap, stop strings)."""
        return {k: v for k, v in self.ai_record().items() if k in ("max_tokens", "stop", "temperature")}

    def start(self) -> "OpenAiPlug":
        return self

    def totals(self) -> dict:
        """What the run is known to have used: the model; no counts (no usage verb)."""
        return {"model": self.model_name, "requests": None, "replies": None, "errors": 0,
                "input_tokens": None, "output_tokens": None, "usd_estimate": None}

    def stop(self) -> None:
        if self._stopped:
            return
        self._stopped = True
        try:
            with open(os.path.join(self.run_dir, "cost.json"), "w") as f:
                json.dump(self.totals(), f, indent=2)
                f.write("\n")
        except OSError:
            pass

    def finish(self, team) -> None:
        """Nothing to read before the team stops: the plug keeps no counts."""

    consumed = None  # unknown: the plug cannot count
    errored = False
    capped = False  # no request cap: nothing ever stops the run
    errored_reason = None


def make(test_dir: str, variables: dict) -> OpenAiPlug:
    why = problem()
    if why:
        raise RuntimeError(f"the openai plug cannot run: {why}")
    return OpenAiPlug(variables)
