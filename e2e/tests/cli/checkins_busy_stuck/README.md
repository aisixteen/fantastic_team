# cli/checkins_busy_stuck

A lead (with the bundles registry; the bundle `bundles/sublead/bundle.yaml`) spawns two sub-leads and looks at them on two check-ins in a row (#284: a generating agent is busy, never stuck; one that is not generating and adds no row across two check-ins is stopped and reported as `task failed: …`).

The lead fixes `muler.py` itself and commits, then spawns `s1` and `s2` (the lead's `send` after each spawn carries the lane marker). `s1`'s first turn is slow (`delay` 8 s): it is `generating` while the lead checks. `s2` awaits with nothing in flight: its run ends, so it is not generating and adds no row. The lead's two check-ins are its own scripted turns (a scheduler tick wakes it as a notice, which the plug answers outside the script): each reads `status` of both and `history` of `s2` (`{{minted:N}}` are the ids from the spawn replies). `s1` is generating both times and is left alone; `s2` is not, and its second `history` reply is the first's, so the lead stops it. `s1` then fixes `adder.py`, commits and reports to the lead with a plain `send`; the lead, awaiting, deletes both and says `task failed: …` with `done`.

Checks: solved; the slow sub-lead read `generating` on both check-ins and was not stopped; the idle one read not generating on both, had no new row, and was stopped; `s1`'s report reached the lead and gave it a turn; the lead said `task failed:`; the records are the ones it started with, the registry counts none running; every lane asked the model; a check-in reached the lead.

Mutation: make the lead `stop` `s1` as well (a `stop` turn on `{{minted:1}}`) and "the lead did not stop the generating one" fails; drop its `stop` of `s2` and "the lead stopped the idle sub-lead" fails.
