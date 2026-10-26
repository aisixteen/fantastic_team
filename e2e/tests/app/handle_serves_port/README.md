# app/handle_serves_port (free)

`TeamHandle::start` with a port serves the dispatcher page in the same process: no child process, and stop releases the port and the directory lock.

The handle is driven by `crates/team/examples/handle_serve.rs` (a test driver, no product API: `--dir D --port P`, prints `url: ...`, stops on end of stdin or SIGTERM). The probe boots the team once with the real binary so its tree is on disk, stops it, and runs the example on the directory. A Skip when the example is not built next to `--bin` (`cargo build -p fantastic-team --example handle_serve`).

Asserts: the example prints the page URL; the page answers 200 and is the dispatcher page with the socket shim; another file of the page is a 404; the page's socket (`/team_ws/ws`) reaches the dispatcher (`sidebar` answers the groups) and nothing outside its grant; the example has no child process and there is no other process on the directory; a second team cannot start on the directory while the handle holds the lock; on end of stdin the example exits 0, the port refuses connections, and a normal team then starts on the directory.

Note: the handle's kernel registry (`fantastic_team::registry()`) has no model backend, credential store or memory store, so the lead of the tree is skipped ("bundle anthropic_backend.tools not installed in this runtime"). A chat with a model through the handle is therefore not possible yet; `dispatcher/chat_*` use the real binary.

Mutation: run the example without `--port`; it prints no URL and the page does not answer.
