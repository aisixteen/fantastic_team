# cli_ssh/single_lead

The ssh twin of `cli/single_lead` (#242, #252): one task split across the local kernel (the master) and a remote one in a Podman box, wired the weak way.

One lead (the only backend, on the master), its workspace split: locally it fixes adder.py, commits and pushes; through the bridge it drives the remote kernel's exec agent in the box's clone (pull, fix muler.py, test, commit, push). Played twice, as `cli/single_lead`: A (the lead laid as a record file) and B (fantastic's recipe, the root AI `ai`), each with a box of its own. No remote agent asks a backend here (the lead is the only one), so there is no stream to check.

Checks: The local checks of `cli/single_lead` for each part; the remote runs' replies came back through the bridge with exit 0; the box holds no backend.

The wiring, the shared checks (`Box.end`) and what the twin needs (Podman, else `SKIP`; builds the Linux binary once per source tree) are the twins' in `e2e/README.md` ("The weak wiring", "The twins"). The scripted plug's `script.json` is written by `python3 -m helpers.ssh_twins --write` in `e2e/`; the fake forge, no network.
