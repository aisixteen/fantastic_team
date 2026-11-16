// THE CONNECTOR VIEW FOLLOWS ITS AGENT (team#351): run whole (the shared harness), it watches
// the connector and re-reads `reflect` on each `call_served` the connector publishes, and on
// nothing else: no timer. Run: `node --test crates/github_connector/tests`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { runView, verbsSince } from "../../agent_view/tests/view_harness.mjs";

const VIEW = fileURLToPath(new URL("../src/view.html", import.meta.url));
const ME = "github";
let calls = [];
const reply = (_t, p) => (p.type === "reflect"
  ? { id: ME, auth: { mode: "local" }, repos: ["acme/widgets"], api_url: "https://api.github.com", recent_calls: calls }
  : { state: "open" });
const run = () => runView(VIEW, ME, reply, { verbs: JSON.stringify(["issues", "pr_status"]) });

test("the view watches its own connector once and sets no timer", async () => {
  const v = run();
  await v.settle();
  assert.deepEqual(v.watches.map((w) => w.src), [ME]);
  assert.deepEqual(v.timers, [], "a view that sets a timer polls");
  assert.deepEqual(verbsSince(v, 0, ME), ["reflect"]);
});

test("a call_served re-reads the connector: its new call is drawn", async () => {
  calls = [];
  const v = run();
  await v.settle();
  assert.match(v.document.getElementById("pg").textContent, /No calls yet\./);
  calls = [{ verb: "issues", target: "acme/widgets", result: "ok" }];
  const before = v.sends.length;
  v.deliver(ME, { type: "call_served", verb: "issues", target: "acme/widgets", result: "ok" });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), ["reflect"]);
  const text = v.document.getElementById("pg").textContent;
  assert.doesNotMatch(text, /No calls yet\./);
  assert.match(text, /issuesacme\/widgetsok/);
});

test("a mirrored call (its own reflect or pr_status) or a refusal reads nothing", async () => {
  const v = run();
  await v.settle();
  const before = v.sends.length;
  v.deliver(ME, { type: "reflect" });
  v.deliver(ME, { type: "pr_status", repo: "acme/widgets", number: 7 });
  v.deliver(ME, { error: "the view may not call github watch", reason: "denied" });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, ME), []);
});
