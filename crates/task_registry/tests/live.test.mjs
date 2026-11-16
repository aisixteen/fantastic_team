// THE TASK REGISTRY VIEW FOLLOWS ITS AGENT (team#351): run whole (the shared harness), it
// watches the registry and re-reads `list` on each `task_changed` the registry publishes,
// and on nothing else: no timer, and a call mirrored to the watcher is not an event.
// Run: `node --test crates/task_registry/tests`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { runView, verbsSince } from "../../agent_view/tests/view_harness.mjs";

const VIEW = fileURLToPath(new URL("../src/view.html", import.meta.url));
const ME = "tasks";
let tasks = [{ id: "o/r#1", status: "waiting", holder: null }];
const reply = (_t, p) => (p.type === "list" ? { tasks, usage: {} } : p.type === "get" ? tasks[0] : {});

test("the view watches its own registry once and sets no timer", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  assert.deepEqual(v.watches.map((w) => w.src), [ME]);
  assert.deepEqual(v.timers, [], "a view that sets a timer polls");
  assert.deepEqual(verbsSince(v, 0, ME), ["list"], "one read when it opens");
});

test("a task_changed re-reads the list and draws what the registry now holds", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  assert.equal(v.document.getElementById("sub").textContent, "1 task · 0 held · 0 blocked");
  tasks = [{ id: "o/r#1", status: "blocked", holder: null }, { id: "o/r#2", status: "running", holder: "w" }];
  const before = v.sends.length;
  v.deliver(ME, { type: "task_changed", change: "status", task: "o/r#1" });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), ["list"]);
  assert.equal(v.document.getElementById("sub").textContent, "2 tasks · 1 held · 1 blocked");
});

test("a call mirrored to the watcher, a refusal or another agent's event reads nothing", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  const before = v.sends.length;
  v.deliver(ME, { type: "list" });
  v.deliver(ME, { type: "add", id: "x", source: "s" });
  v.deliver(ME, { error: "the view may not call tasks watch", reason: "denied" });
  v.deliver("other", { type: "task_changed", change: "add", task: "x" });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), []);
});

test("events that land while a read is out are read once more when it is back", async () => {
  let release;
  const gate = new Promise((r) => { release = r; });
  let first = true;
  const slow = (t, p) => {
    if (p.type === "list" && first) { first = false; return gate.then(() => ({ tasks, usage: {} })); }
    return reply(t, p);
  };
  const v = runView(VIEW, ME, slow);
  v.deliver(ME, { type: "task_changed", change: "add", task: "a" });
  v.deliver(ME, { type: "task_changed", change: "add", task: "b" });
  assert.deepEqual(verbsSince(v, 0, ME), ["list"], "one read at a time");
  release();
  await v.settle();
  assert.deepEqual(verbsSince(v, 0, ME), ["list", "list"], "the events missed are one more read");
});
