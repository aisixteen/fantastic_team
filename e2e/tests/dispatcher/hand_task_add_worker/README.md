# dispatcher/hand_task_add_worker (free)

Group `g` has `enabled: false`. The probe adds `w2`, hands it a task and removes it.

Asserts: `add_worker` returns `id: w2`; the dispatcher's tree lists it under `g`; `w2` is live and has no `w2_memory` (it names the team's one store `memory`); the board's rows put it under `g`. `hand_task` is sent and the scripted model gets a request holding `task fix issue 1`. `remove_worker` without `confirm` asks first; with it, the worker, its memory, its tree row and its board row are gone.

Mutation: skip `save_groups` in `add_worker`, or drop `task {task}` from the `hand_task` send (`crates/team/src/dispatcher.rs`), and the test fails.
