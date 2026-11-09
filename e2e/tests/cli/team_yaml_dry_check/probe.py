"""A preset yaml: a missing field fails naming it; a whole one is applied and passes the dry check."""

import os
import subprocess

from helpers import team as team_mod
from helpers.verdict import Verdict

GOOD = """credentials:
  model/{model_host}:
    host: {model_host}
    env: {{api_key: FANT_AK}}
  github/app:
    host: {forge_host}
    env: {{app_id: FANT_GH_APP_ID, private_key: FANT_GH_APP_KEY}}
provider:
  endpoint: {model_endpoint}
  caching: provider_cache
  models: [{model_name}]
  default_model: {model_name}
  credentials: model/{model_host}
connectors:
  - repo: acme/widgets
    clone: clone
    app: github/app
    api_url: {forge_url}
    remote: {forge_url}/acme/widgets.git
    author: {{name: "team-bot[bot]", email: team-bot@example.invalid}}
worktrees: work/wt
roles:
  limits: {{total: 4, per_role: {{worker: 3}}}}
lead:
  model: {model_name}
"""


def run_binary(ctx, folder, env, *args, timeout=60):
    return subprocess.run([ctx.binary, *args], cwd=folder, env=env, capture_output=True, text=True,
                          timeout=timeout, stdin=subprocess.DEVNULL)


def run(ctx):
    v = Verdict("cli/team_yaml_dry_check")
    keys = {"FANT_AK": "", "FANT_GH_APP_ID": "", "FANT_GH_APP_KEY": ""}
    from helpers import run_flow
    k = run_flow.fake_keys(ctx)
    keys.update(FANT_AK=k["model_key"], FANT_GH_APP_ID=k["app_id"], FANT_GH_APP_KEY=k["pem"])
    text = GOOD.format(**ctx.vars)

    # a missing field: named, nothing written
    bad = os.path.join(ctx.run_dir, "bad")
    os.makedirs(bad)
    with open(os.path.join(bad, "team.yaml"), "w") as f:
        f.write(text.replace(f"  endpoint: {ctx.vars['model_endpoint']}\n", ""))
    env = team_mod.Team(ctx.binary, bad, keys).env
    for args in (("--headless",), ("check",)):
        r = run_binary(ctx, bad, env, *args)
        v.check(f"`{' '.join(args)}` fails naming provider.endpoint",
                r.returncode != 0 and "provider.endpoint" in r.stderr, f"{r.returncode}: {r.stderr[-300:]}")
    v.check("nothing was written for the bad preset", not os.path.exists(os.path.join(bad, ".fantastic")))

    # a whole preset: the dry check applies the fresh folder and passes
    team = team_mod.Team(ctx.binary, ctx.run_dir, keys)
    ctx.team = team
    with open(os.path.join(ctx.run_dir, "team.yaml"), "w") as f:
        f.write(text)
    r = run_binary(ctx, ctx.run_dir, team.env, "check")
    out = r.stdout
    v.check("the dry check passes", r.returncode == 0 and "FAIL" not in out, f"{r.returncode}: {out[-600:]} {r.stderr[-300:]}")
    for want in ("caching is provider_cache", "roots are under the team folder", "rooted at the worktree root",
                 "roles limit 4 in all", "whoami: app"):
        v.check(f"the dry check reports: {want}", f"ok   " in out and want in out, out[-600:])
    v.check("the forge saw the app's whoami", bool(ctx.forge.to("GET", "/app")))
    v.check("no model call was made", ctx.model.served == 0, f"served {ctx.model.served}")

    # the team starts on the seeded records
    team.start()
    a = team.agents()
    v.check("the provider keeps `caching: provider_cache`", a["anthropic"].get("caching") == "provider_cache")
    v.check("the lead runs on the provider and the model",
            a["lead"].get("provider_id") == "anthropic" and a["lead"].get("model") == ctx.vars["model_name"])
    v.check("the connector pair names the repo", a["github"].get("repos") == ["acme/widgets"]
            and a["github"].get("git") == "git" and a["git"].get("repo_dir", "").startswith(ctx.run_dir))
    v.check("the roles' limits are the preset's", (a["roles"].get("limits") or {}).get("total") == 4)
    v.check("the credential store holds names, never values",
            a["credentials"]["refs"]["github/app"]["env"]["private_key"] == "FANT_GH_APP_KEY")
    team.stop()
    held = [os.path.join(d, f) for d, _, fs in os.walk(ctx.run_dir) for f in fs
            if "/forge/" not in os.path.join(d, f) and "/secrets/" not in os.path.join(d, f)
            and "/clone/" not in os.path.join(d, f) and "/home/" not in os.path.join(d, f)
            and any(n.encode() in open(os.path.join(d, f), "rb").read() for n in (k["model_key"], k["pem_chunk"]))]
    v.check("no file of the team folder holds a key", not held, str(held))
    return v.finish()
