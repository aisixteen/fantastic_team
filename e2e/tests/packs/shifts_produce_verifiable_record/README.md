# packs/shifts_produce_verifiable_record (free)

Issue #41: a team's real shifts produce a measured record that is signed and verifies.

The team has a `board` section and the fake forge lists two open issues. The probe writes the queue state files the board reads (`in_work lead`, then `done acme/widgets#7` for one and `failed` for the other) and pulls. `pack eval {from_shifts: true, signing_key}` then replies with a record of 2 tasks, 1 success, rate 0.5, signed; the same eval without `from_shifts` counts the eval run (1 task), so the flag matters. `pack_listing publish {opt_in: true}` accepts the signed record (the signature verifies); a record with one field changed (`success_rate`) is refused with a reason saying it does not verify.
