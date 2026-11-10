"""Plugs: what answers the team's model calls.

A test never names a model, an endpoint or a key; its test.yaml says only which
plug (`plug: scripted` by default). A plug has `make(test_dir, variables)` and
returns an object with `.endpoint`, `.host`, `.start()`, `.stop()`, and:
  `.model_name`  what the lead's `model` field takes (`provides_vars`);
  `.key_env`     the variable holding the real key, or None (the launcher reads it
                 for a real plug, and never writes it to disk);
  `.real`        true when a real model answers.

Three plugs exist:
  `scripted`  a fake of the Messages wire that plays a script: free, no network.
  `anthropic` a recording relay to a real model (aisixteen/fantastic#302): the turns
              tier, run by hand, skipped without E2E_ANTHROPIC_KEY or ANTHROPIC_API_KEY.
  `vllm`      a vLLM server reached directly, no relay (the gpu tier, `backends/vllm.py`): run by
              hand, skipped without VLLM_ENDPOINT, FANTASTIC_AI_MODEL, VLLM_KV_SLOTS, VLLM_KV_OVERFLOW.
"""

import json

from . import anthropic, scripted, vllm


def _scripted(test_dir, variables):
    return scripted.load(f"{test_dir}/script.json", variables)


def _anthropic(test_dir, variables):
    return anthropic.make(test_dir, variables)


def _vllm(test_dir, variables):
    return vllm.make(test_dir, variables)


PLUGS = {"scripted": _scripted, "anthropic": _anthropic, "vllm": _vllm}
#: Why a plug cannot run here (None when it can): a real plug needs its key or its server's variables.
WHY_NOT = {"scripted": lambda: None,
           "anthropic": lambda: None if anthropic.available() else "no key in the environment",
           "vllm": vllm.problem}


def why_not(name: str) -> str | None:
    return WHY_NOT[name]()


def available(name: str) -> bool:
    """True when the plug can run here (a real plug needs its key or its variables in the environment)."""
    return why_not(name) is None


SHAPES = {"anthropic_backend.tools": "anthropic", "vllm_backend.tools": "vllm"}


def provides_vars(plug) -> dict:
    """The variables a plug gives a test (`ctx.vars`): its model name, the shape of its provider
    (`anthropic`, `vllm`, `vllm_keyed`: the name of a fleet layer) and the provider's fields as text
    ready for a record file (JSON; `null` when the plug has none), all read from `ai_record()`."""
    rec = plug.ai_record()
    shape = SHAPES[rec["handler_module"]]
    if shape == "vllm" and plug.keyed:
        shape = "vllm_keyed"
    out = {"model_name": plug.model_name, "provider_shape": shape, "kv_overflow": rec.get("kv_overflow", "")}
    for field in ("caching", "max_tokens", "kv_slots", "no_repeat_ngram_size", "repetition_penalty"):
        out[field] = json.dumps(rec.get(field))
    return out
