# dispatcher/hand_task_add_worker (free)

Group `g` has `enabled: false`. The probe adds `w2`, hands it a task and removes it.

Asserts: `add_worker` returns `id: w2`; the tree the page's sidebar draws (the root's full `reflect`) lists it; the dispatcher's record lists it in group `g` beside `w1` (a member with no agent of its own here, so not in the tree); `w2` is live and has no `w2_memory` (it names the team's one store `memory`). `hand_task` is sent and the scripted model gets a request holding `task fix issue 1`. `remove_worker` without `confirm` asks first; with it, the worker, its memory, its tree row and its place in `g` are gone.

Mutation: skip `save_groups` in `add_worker`, or drop `task {task}` from the `hand_task` send (`crates/team/src/dispatcher.rs`), and the test fails.
