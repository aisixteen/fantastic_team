// THE BUNDLES REGISTRY VIEW FOLLOWS ITS AGENT (team#351): run whole (the shared harness), it
// watches the registry and re-reads its kinds, fleet and count on each `fleet_changed` the
// registry publishes, and on nothing else: no timer. Run: `node --test crates/agentic_resources/tests`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { runView, verbsSince } from "../../agent_view/tests/view_harness.mjs";

const VIEW = fileURLToPath(new URL("../src/view.html", import.meta.url));
const ME = "bundles";
let running = 0;
const reply = (_t, p) => ({
  bundles: { bundles: [{ name: "worker", purpose: "works" }], errors: [] },
  fleet: { bundles: { worker: Array.from({ length: running }, (_, i) => "w" + i) } },
  get_count: { running },
  bundle: { name: "worker", directive: "work" },
}[p.type] || {});

test("the view watches its own registry once and sets no timer", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  assert.deepEqual(v.watches.map((w) => w.src), [ME]);
  assert.deepEqual(v.timers, [], "a view that sets a timer polls");
  assert.deepEqual(verbsSince(v, 0, ME).sort(), ["bundle", "bundles", "fleet", "get_count"]);
});

test("a fleet_changed re-reads the kinds, the fleet, the count and the picked card", async () => {
  running = 0;
  const v = runView(VIEW, ME, reply);
  await v.settle();
  assert.equal(v.document.getElementById("run").textContent, "0 running");
  running = 2;
  const before = v.sends.length;
  v.deliver(ME, { type: "fleet_changed", change: "added", agent: "w1", bundle: "worker" });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME).sort(), ["bundle", "bundles", "fleet", "get_count"]);
  assert.equal(v.document.getElementById("run").textContent, "2 running");
});

test("a mirrored call (spawn among them) or a refusal reads nothing", async () => {
  const v = runView(VIEW, ME, reply);
  await v.settle();
  const before = v.sends.length;
  v.deliver(ME, { type: "fleet" });
  v.deliver(ME, { type: "spawn", bundle: "worker" });
  v.deliver(ME, { error: "the view may not call bundles watch", reason: "denied" });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), []);
});
