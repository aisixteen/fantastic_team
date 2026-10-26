# dispatcher/digest_reaches_driver (free)

Group `g` (driver `lead`, members `w1`, `w2`) and a board. The probe sets the cadencer's cadence to 2s; it never sends a `tick`.

Asserts: a model request reaches the driver holding the digest (`w1: idle`, `w2: idle`, the `load:` line); the cadencer's `status` shows `ticks` and `digests_sent` of at least 1, `delivery_errors` 0 and a metered `digest_bytes_per_hour`; the team log has no `cadencer call tick`.

Mutation: return early in `restart_timer` (`crates/team/src/cadencer.rs`) before the timer is spawned and the test fails.
