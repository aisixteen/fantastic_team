# dispatcher/quiet_heartbeat (free)

Quiet mode is on (the default). Bundles mode as in `dispatcher/digest_reaches_driver`: a spawned `project_lead` whose scripted turns spawn two workers through `bundles`, both idle. The probe adds one waiting task to the task registry (`tasks add`, no holder) and sets the cadence to 1s.

Asserts: no heartbeat before the cadence is set; the first beat is a regular digest naming both workers; `heartbeat: 2 idle, 1 queued` reaches the lead after at least four ticks (three quiet beats), and the regular digest is not a heartbeat; `digests_sent` is at least 2 and `delivery_errors` 0.

Mutation: drop the `heartbeat(...)` call in `beat` (`crates/team/src/cadencer.rs`) and the test fails.
