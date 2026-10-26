# dispatcher/stop_asks_then_stops (free)

Group `g` has `enabled: false`, so no digest adds noise. The probe adds worker `w2` through `control add_worker`.

Asserts: `control stop` without `confirm` answers with a question (`Stop w2`), `sent: false`, and the team log shows no `w2 call stop`; the worker is still listed. With `confirm: true` it is sent with no error, the log shows `w2 call stop`, and the worker's `status` is not generating. (The scripted model has no delay, so the stop is seen through the call and the status.)

Mutation: remove the `stop` without `confirm` branch of `control_reply` (`crates/team/src/dispatcher.rs`) and the first check fails.
