# cli_ssh/checkins_busy_stuck

The ssh twin of `cli/checkins_busy_stuck` (#284): a lead's check-ins leave a generating sub-lead alone and stop an idle one, across a local kernel (the master) and a remote one in a Podman box, wired the weak way (see `e2e/README.md`).

The lead fixes `muler.py` in the box itself (the remote exec agent through the bridge: pull, fix, test, commit, push), then spawns `s1` and `s2` on the master's roles registry. `s1`'s first turn is slow (`BUSY_S`), so it is `generating` the whole time; then it pulls, fixes `adder.py`, pushes and reports to the lead with a plain `send`. `s2` awaits with nothing in flight: its run ends, it is not generating and adds no row. The lead's two check-ins are its own scripted turns (a scheduler tick is a notice the plug answers outside the script): each reads `status` of both and `history` of `s2`. `s1` is generating both times and is left alone; `s2` is not, and its second `history` reply is the first's (no new row), so the lead stops it, waits for `s1`'s report, deletes both and says `task failed: …`.

Checks: the `cli/checkins_busy_stuck` ones (the slow sub-lead read `generating` on both check-ins and was not stopped; the idle one read not generating with no new row on both and was stopped; `s1`'s report reached the lead and gave it a turn; the lead said `task failed:`; both deleted, nothing running) and `Box.end`'s shared ones (solved on the git server with a commit from each side, the bridge, the tunnel, the remote kernel, the container removed).

Needs Podman with a running machine (else `SKIP`). Free: the scripted plug (`script.json`, written by `python3 -m helpers.ssh_twins --write` in `e2e/`), the fake forge, no network.
