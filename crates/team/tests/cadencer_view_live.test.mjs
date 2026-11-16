// THE CADENCER VIEW FOLLOWS ITS AGENT (team#351): run whole (the shared harness), it watches
// the cadencer and re-reads `status` on each `settings_changed` and `ticked` the cadencer
// publishes, and on nothing else: no timer. A number being typed is not redrawn under the
// cursor: the read waits until the field is left. Run: `node --test crates/team/tests`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { runView, verbsSince } from "../../agent_view/tests/view_harness.mjs";

const VIEW = fileURLToPath(new URL("../src/cadencer/view.html", import.meta.url));
const ME = "cadencer";
let enabled = false;
const reply = (_t, p) => (p.type === "status"
  ? { settings: { enabled, quiet: false, cadence: 60, stall_after: 300, off: [] }, ticks: 0, digests_sent: 0, delivery_errors: 0, load: {} }
  : { enabled });

test("the view watches its own cadencer once and sets no timer", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  assert.deepEqual(v.watches.map((w) => w.src), [ME]);
  assert.deepEqual(v.timers, [], "a view that sets a timer polls");
  assert.deepEqual(verbsSince(v, 0, ME), ["status"]);
});

test("settings_changed and ticked re-read the status; the switch then reads what the cadencer holds", async () => {
  enabled = false;
  const v = runView(VIEW, ME, reply);
  await v.settle();
  assert.equal(v.document.getElementById("state").textContent, "off");
  enabled = true;
  let before = v.sends.length;
  v.deliver(ME, { type: "settings_changed", settings: { enabled: true } });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), ["status"]);
  assert.equal(v.document.getElementById("state").textContent, "on");
  before = v.sends.length;
  v.deliver(ME, { type: "ticked", ticks: 1, sent: 0, delivery_errors: 0 });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), ["status"]);
});

test("a mirrored call or a refusal reads nothing", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  const before = v.sends.length;
  v.deliver(ME, { type: "status" });
  v.deliver(ME, { type: "set", quiet: true });
  v.deliver(ME, { error: "the view may not call cadencer watch", reason: "denied" });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), []);
});

test("an event while a number is typed waits until the field is left", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  const input = v.document.createElement("input");
  input.focus();
  const before = v.sends.length;
  v.deliver(ME, { type: "ticked", ticks: 2, sent: 1, delivery_errors: 0 });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), [], "not redrawn under the cursor");
  input.blur();
  v.document.getElementById("pg").dispatch("focusout");
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), ["status"], "read once the field is left");
});
