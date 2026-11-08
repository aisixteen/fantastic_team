"""cli_ssh/single_lead: one lead, its workspace split across the local kernel and a remote one in a
Podman box, started two ways (A: `create_agent` under the id `lead`; B: fantastic's recipe, the root
AI `ai`), each with a box of its own. Locally the lead fixes adder.py, commits and pushes; through the
bridge it drives the remote kernel's exec agent in the box's clone: pull, fix muler.py, test, commit,
push. The wiring and the shared checks are `helpers/ssh_twins.py`'s; the local checks are
`cli/single_lead`'s (`helpers/cli_scenarios.py`)."""

from helpers import cli_scenarios as S, podman_remote, ssh_twins as T
from helpers.verdict import Verdict

TASK = "Fix the failing test: adder.py here, muler.py in your workspace on the remote box; push both."


def part(ctx, v, label: str, start, upto: int) -> None:
    with T.Box(ctx, "box_" + label.lower()) as box:
        lead, before = start(ctx, v)
        box.publish()
        box.bind()
        v.check(f"{label}: it played every turn of its part", T.hand(ctx, lead, TASK, upto=upto),
                f"served {ctx.model.served} of {upto}")
        S.single_end(ctx, v, label, before)
        replies = T.forward_replies(ctx)[-5:]
        v.check(f"{label}: the remote workspace's runs came back through the bridge (pull, fix, test, commit, push: exit 0)",
                len(replies) == 5 and all("<exit_code>0</exit_code>" in r for r in replies),
                str([r[:200] for r in replies]))
        box.end(v, label=f"{label}: ")
        ctx.team.stop()
    box.removed(v, f"{label}: ")


def run(ctx):
    v = Verdict("cli_ssh/single_lead")
    podman_remote.require()
    per_run = len(ctx.model.turns) // 2
    part(ctx, v, "A", S.single_by_hand, per_run)
    # B (fantastic's `ai anthropic` recipe) waits for fantastic#518: the recipe still makes a
    # provider as the root chat, and a provider holds no conversation now
    print("  B: skipped until fantastic#518 (the recipe makes a provider-only root)")
    v.check("the scripted model played exactly A's turns, none past them",
            ctx.model.served == per_run and not ctx.model.overrun, f"served {ctx.model.served} of {per_run}")
    return v.finish()
