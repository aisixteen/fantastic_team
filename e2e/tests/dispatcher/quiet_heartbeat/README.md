# dispatcher/quiet_heartbeat (free)

Quiet mode is on (the default). `board pull` queues the forge's open issue, both workers are idle, and the probe sets the cadence to 1s.

Asserts: no heartbeat before the cadence is set; the first beat is a regular digest; `heartbeat: 2 idle, 1 queued` reaches the driver after at least four ticks (three quiet beats), and the regular digest is not a heartbeat. The script has spare turns; `consumed` is not asserted.

Mutation: drop the `heartbeat(...)` call in `beat` (`crates/team/src/cadencer.rs`) and the test fails.
