"""cli_ssh/git_credentials (#253): the remote box gets its git credentials through the kernel bridge, from the
MASTER's credential store, per operation, and stores none.

The master holds the git server's token in a credential store of its own (`git_credentials`, one ref bound to the
server's host; its `env` backend keeps no value at rest). The box has no token: not in a store, an environment
variable, a file or a git config, from the start to the end. The git connector (`team_git.tools`) runs on the REMOTE
kernel and its `credentials` name the master's store as `<master guid>|git_credentials`: each fetch and push reads the
token over the bridge (`get`, the reply marked `secret` on the wire), hands it to the one-shot credential helper's
environment and drops it. On the box: fetch, a worktree of origin/main, the fix, commit, fetch, push a work branch (it
lands on the git server). Message 2: the store's grant revoked and a push over the open bridge (refused), the grant
given back and the push again (lands), the bridge deleted and a push by the box's own `send` (refused). The wiring and
the shared checks are `helpers/ssh_twins.py`'s."""

import os
import subprocess

from helpers import cli, forge as F, podman_remote, ssh_twins as T
from helpers.verdict import Verdict

SETUP = "Set up the remote box: the kernel, the bridge, and a git connector there that reads its token from our store."
TASK = "Fix muler.py in the box's workspace with the git connector there: fetch, a work branch, commit, push."
CLOSE = "Check the grants: push again with the store's grant revoked and given back, then close the remote."


def server_refs(ctx) -> dict:
    """The test's git server: branch -> commit."""
    bare = os.path.join(ctx.run_dir, "forge", *T.git_url(ctx).rsplit("/", 2)[-2:])
    out = subprocess.run(["git", "-C", bare, "for-each-ref", "--format=%(objectname) %(refname:short)", "refs/heads"],
                         capture_output=True, text=True, check=True).stdout
    return {line.split()[1]: line.split()[0] for line in out.splitlines()}


def server_git(ctx, *args: str) -> str:
    """A git command on the test's git server's repository."""
    bare = os.path.join(ctx.run_dir, "forge", *T.git_url(ctx).rsplit("/", 2)[-2:])
    return subprocess.run(["git", "-C", bare, *args], capture_output=True, text=True).stdout


def count(replies, text: str) -> int:
    return sum(text in r for r in replies)


def run(ctx):
    v = Verdict("cli_ssh/git_credentials")
    podman_remote.require()
    n = T.phases_git()
    token = ctx.forge.token
    with T.Box(ctx, git_token=False) as box:
        cli.start(ctx, env={T.GIT_ENV: token})
        lead = cli.LEAD
        before = cli.ids(ctx.team)
        box.publish()
        start = server_refs(ctx)
        seen = len(ctx.forge.requests)  # the harness's own fetch (publish) is before this
        box.bind()
        guid = T.master_guid(ctx.team)

        # 1. the master holds the token in a store bound to the git server's host; the box holds none
        store = (ctx.team.send(T.GIT_STORE, {"type": "list"}).get("result") or {}).get("refs") or []
        v.check("1: the master's store holds the git server's token as one ref, bound to the server's host",
                [(r.get("ref"), r.get("host")) for r in store] == [(T.GIT_REF, ctx.forge.host)], str(store))
        v.check("1: the box has no credential store at start (its fresh kernel's records hold none)",
                not [i for i, a in box.stock_records.items() if a.get("handler_module") == "credential_store.tools"],
                str(sorted(box.stock_records)))

        # 2. the connector on the remote names the master's store
        v.check("2: the scripted lead played message 1 (the box, the bridge, the git connector on the remote kernel)",
                T.hand(ctx, lead, SETUP, upto=n["setup"]), f"served {ctx.model.served} of {n['setup']}")
        # 3. on the remote: fetch, worktree, fix, commit, fetch, push: the token read from the master's store, per operation
        v.check("3: the scripted lead played message 2 (fetch, worktree, fix, commit, fetch, push, on the remote)",
                T.hand(ctx, lead, TASK, upto=n["setup"] + n["work"]), f"served {ctx.model.served} of {n['setup'] + n['work']}")
        remote = T.remote_agents(box.remote_call("kernel", {"type": "list_agents"}))
        g = remote.get(T.GIT) or {}
        v.check("3: the connector on the remote kernel is a `team_git` agent whose credentials name the MASTER's store "
                "(`<master guid>|git_credentials`) and the ref: a name and a ref, no value",
                g.get("handler_module") == "team_git.tools"
                and g.get("credentials") == {"store": f"{guid}|{T.GIT_STORE}", "ref": T.GIT_REF}, str(g)[:300])
        v.check("3: the remote kernel has no credential store",
                not [i for i, a in remote.items() if a.get("handler_module") == "credential_store.tools"], str(sorted(remote)))
        replies = T.forward_replies(ctx)
        for what, want, times in (("fetch (twice)", "<fetched>true</fetched>", 2), ("worktree_add", "<created>true</created>", 1),
                                  ("commit", "<committed>true</committed>", 1), ("push", "<pushed>true</pushed>", 1)):
            v.check(f"3: remote `{what}` answered through the bridge", count(replies, want) == times,
                    str([r[:160] for r in replies]))
        refs = server_refs(ctx)
        top = refs.get(T.BRANCH, "")
        files = server_git(ctx, "show", f"{top}:muler.py") if top else ""
        v.check("3: the push landed on the git server: a branch work/muler, one commit on top of main (muler.py fixed, the "
                "author the connector's), main untouched",
                set(refs) == {"main", T.BRANCH} and refs["main"] == start["main"]
                and server_git(ctx, "rev-parse", f"{top}^").strip() == start["main"] and "a * b" in files
                and server_git(ctx, "log", "-1", "--format=%an <%ae>", top).strip() == "%s <%s>" % T.AUTHOR, f"{refs} {files!r}")
        authed = [r for r in ctx.forge.requests[seen:] if r["path"].startswith(f"/{F.REPO}.git/") and r.get("authorized")]
        pushes = [r for r in authed if r["path"].endswith("/git-receive-pack")]
        v.check("3: the token reached the git server only as git sends it: authorized requests to its git endpoint "
                "(the box's fetches and the push), and no request outside it carried an Authorization header",
                len(pushes) == 1 and len(authed) >= 4 and not [
                    r for r in ctx.forge.requests if not r["path"].startswith(f"/{F.REPO}.git/")
                    and r["headers"].get("authorization")], f"{len(authed)} authorized, {len(pushes)} push(es)")

        # 5. the grant revoked, then the bridge closed: a clear refusal each time
        v.check("5: the scripted lead played message 2 (grant revoked, push refused, grant back, push; bridge deleted, push "
                "refused)", T.hand(ctx, lead, CLOSE) and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        replies = T.forward_replies(ctx)
        refused = [r for r in replies if "<error>" in r and T.GIT_STORE in r]
        v.check("5: with the store's grant revoked a push over the open bridge is refused, naming the store and the ref",
                len(refused) == 1 and T.GIT_REF in refused[0] and "<branch>" not in refused[0], str(refused)[:400])
        v.check("5: the grant given back, the same push goes through", count(replies, f"<branch>{T.REVOKED}</branch>") == 1
                and count(replies, "<pushed>true</pushed>") == 2, str([r[:100] for r in replies[-4:]]))
        v.check("5: with the bridge deleted, a push by the box's own `send` fails: the remote kernel has no agent "
                f"`{T.GIT_STORE}` to ask (a clear refusal)",
                T.came_back(ctx, None, f"no agent '{guid}|{T.GIT_STORE}'", via="worker_exec run"))
        refs = server_refs(ctx)
        v.check("5: the server holds the pushed branches only: work/muler, work/revoked (after the grant came back), not work/closed",
                set(refs) == {"main", T.BRANCH, T.REVOKED}, str(refs))
        gets = [line for line in ctx.team.log().splitlines() if line.endswith(f"{T.GIT_STORE} call get")]
        v.check("5: the master's store was asked once per operation that reached it: 2 fetches and 2 pushes (the refused "
                "ones never read it)", len(gets) == 4, str(gets))
        pushes = [r for r in ctx.forge.requests[seen:] if r["path"].endswith("/git-receive-pack") and r.get("authorized")]
        v.check("5: the git server saw two authorized pushes, none for the refused ones", len(pushes) == 2, str(len(pushes)))
        v.check("the scripted model played exactly its turns, none past the end", ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        v.check("the master's records are the ones it started with (the bridge deleted)", cli.ids(ctx.team) == {
            i: h for i, h in before.items()}, f"{sorted(cli.ids(ctx.team))} vs {sorted(before)}")
        recs = box.end(v, remote_base=(T.GIT,), solved=False, landed=False)
        v.check("remote: the git connector's record is the only trace of the credential: store and ref, never a value",
                (recs.get(T.GIT) or {}).get("credentials") == {"store": f"{guid}|{T.GIT_STORE}", "ref": T.GIT_REF}
                and not [i for i, a in recs.items() if a.get("handler_module") == "credential_store.tools"],
                str(recs.get(T.GIT))[:300])
        ctx.team.stop()
    box.removed(v)
    return v.finish()
