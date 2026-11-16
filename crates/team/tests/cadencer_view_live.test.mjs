// THE CADENCER VIEW FOLLOWS ITS AGENT (team#351): run whole (fantastic's view harness), it watches
// the cadencer and re-reads `status` on each `settings_changed` and `ticked` the cadencer
// publishes, and on nothing else: no timer. A number being typed is not redrawn under the
// cursor: the read waits until the field is left. Run: `node --test crates/team/tests`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { runView, verbsSince, watched, sendsOnOthers, ownCopiesReadNothing } from "../../agent_view/tests/views.mjs";

const VIEW = fileURLToPath(new URL("../src/cadencer/view.html", import.meta.url));
const ME = "cadencer";
let enabled = false;
const reply = (_t, p) => (p.type === "status"
  ? { settings: { enabled, quiet: false, cadence: 60, stall_after: 300, off: [] }, ticks: 0, digests_sent: 0, delivery_errors: 0, load: {} }
  : { enabled });

test("the view watches its own cadencer once and sets no timer", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  assert.deepEqual(watched(v), [[ME, 1]]);
  assert.deepEqual(v.intervals, [], "a view that sets a timer polls");
  assert.deepEqual(verbsSince(v, 0, ME), ["status"]);
});

test("settings_changed and ticked re-read the status; the switch then reads what the cadencer holds", async () => {
  enabled = false;
  const v = runView(VIEW, ME, reply);
  await v.settle();
  assert.equal(v.document.getElementById("state").textContent, "off");
  enabled = true;
  let before = v.sends.length;
  await v.publish(ME, { type: "settings_changed", settings: { enabled: true } });
  assert.deepEqual(verbsSince(v, before, ME), ["status"]);
  assert.equal(v.document.getElementById("state").textContent, "on");
  before = v.sends.length;
  await v.publish(ME, { type: "ticked", ticks: 1, sent: 0, delivery_errors: 0 });
  assert.deepEqual(verbsSince(v, before, ME), ["status"]);
});

test("a mirrored call or a refusal reads nothing", async () => {
  const v = runView(VIEW, ME, reply);
  assert.equal(await sendsOnOthers(v, [
    [ME, { type: "status" }],
    [ME, { type: "set", quiet: true }],
    [ME, { error: "the view may not call cadencer watch", reason: "denied" }],
  ]), 0);
});

test("an event while a number is typed waits until the field is left", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  const input = v.document.createElement("input");
  input.focus();
  const before = v.sends.length;
  await v.publish(ME, { type: "ticked", ticks: 2, sent: 1, delivery_errors: 0 });
  assert.deepEqual(verbsSince(v, before, ME), [], "not redrawn under the cursor");
  input.blur();
  v.document.getElementById("pg").dispatch("focusout");
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), ["status"], "read once the field is left");
});

test("copies of its own calls, and another client's reads, read nothing (the kernel mirrors every call to a watcher)", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  assert.deepEqual(verbsSince(v, 0, ME), ["status"], "its own read's copy reads nothing more");
  await ownCopiesReadNothing(v);
  assert.equal(await sendsOnOthers(v, [[ME, { type: "status" }], [ME, { type: "set", quiet: true }], [ME, { type: "reflect" }]]), 0);
  // a published event is one read, and that read's copy is no more
  const before = v.sends.length;
  await v.publish(ME, { type: "ticked", ticks: 1, sent: 0, delivery_errors: 0 });
  const n = v.sends.length - before;
  assert.ok(n > 0, "the event is read");
  await v.settle();
  assert.equal(v.sends.length - before, n, "no read on the copies of that read");
});
