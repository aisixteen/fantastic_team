"""Plugs: what answers the team's model calls.

A test never names a model, an endpoint or a key; its test.yaml says only which
plug (`plug: scripted` by default). A plug has `make(test_dir, variables)` and
returns an object with `.endpoint`, `.host`, `.start()`, `.stop()`.
Only the free plug exists. The turns tier (a real model through a provider
plug) and the gpu tier (a local-model plug) are not built.
"""

from . import scripted


def _scripted(test_dir, variables):
    return scripted.load(f"{test_dir}/script.json", variables)


PLUGS = {"scripted": _scripted}
