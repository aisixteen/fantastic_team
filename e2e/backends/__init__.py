"""Plugs: what answers the team's model calls.

A test never names a model, an endpoint or a key; its test.yaml says only which
plug (`plug: scripted` by default). A plug has `make(test_dir, variables)` and
returns an object with `.endpoint`, `.host`, `.start()`, `.stop()`, and:
  `.model_name`  what team.yaml's `{{model_name}}` takes (`provides_vars`);
  `.key_env`     the variable holding the real key, or None (the launcher reads it
                 for a real plug, and never writes it to disk);
  `.real`        true when a real model answers.

Two plugs exist:
  `scripted`  a fake of the Messages wire that plays a script: free, no network.
  `anthropic` a recording relay to a real model (aisixteen/fantastic#302): the turns
              tier, run by hand, skipped without E2E_ANTHROPIC_KEY or ANTHROPIC_API_KEY.
The gpu tier (a local-model plug, aisixteen/fantastic#303) is not built.
"""

from . import anthropic, scripted


def _scripted(test_dir, variables):
    return scripted.load(f"{test_dir}/script.json", variables)


def _anthropic(test_dir, variables):
    return anthropic.make(test_dir, variables)


PLUGS = {"scripted": _scripted, "anthropic": _anthropic}
AVAILABLE = {"scripted": lambda: True, "anthropic": anthropic.available}


def available(name: str) -> bool:
    """True when the plug can run here (a real plug needs its key in the environment)."""
    return AVAILABLE[name]()


def provides_vars(plug) -> dict:
    """The variables a plug gives a team.yaml."""
    return {"model_name": plug.model_name}
