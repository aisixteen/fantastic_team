// The team page's pure block (waitingCount, cardOf), run in a bare vm context. `node --test crates/team/tests`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { runView, verbsSince, watched, sendsOnOthers, ownCopiesReadNothing } from "../../agent_view/tests/views.mjs";

const src = readFileSync(fileURLToPath(new URL("../web/team.html", import.meta.url)), "utf8");
const from = src.indexOf("// pure\n");
const to = src.indexOf("// end pure", from);
assert.ok(from >= 0 && to > from);
const ctx = vm.createContext({});
vm.runInContext(`${src.slice(from, to)}\nthis.api = { waitingCount, cardOf, inquiryEvent, pathTo, crumbPath };`, ctx);
const { waitingCount, cardOf, inquiryEvent, pathTo, crumbPath } = ctx.api;

test("waitingCount counts askers once, other than the open agent", () => {
  const inq = [{ asker: "a" }, { asker: "a" }, { asker: "b" }, { asker: "c" }, { text: "x" }, null];
  assert.equal(waitingCount(inq, "c"), 2);
  assert.equal(waitingCount(inq, "zz"), 3);
  assert.equal(waitingCount(undefined, "a"), 0);
  assert.equal(waitingCount("no", "a"), 0);
});

test("cardOf names the agent, tags its handler, says it has no view", () => {
  const c = cardOf("w1", { display_name: "Worker", handler_module: "worker.tools" });
  assert.deepEqual(JSON.parse(JSON.stringify(c)), { name: "Worker", up: "", tag: "worker.tools", note: "This agent has no view of its own." });
  assert.equal(cardOf("w1", {}, "", "Lead").up, "Lead", "the parent's name over it");
  assert.equal(cardOf("w1", {}).name, "w1");
  assert.equal(cardOf("w1", null).tag, "");
});

test("cardOf takes the bundle from the kernel's record when the agent's reflect does not name it", () => {
  // a bridge's reflect carries no handler_module and no display_name (#351)
  const c = cardOf("worker_exec", { id: "worker_exec", sentence: "x" }, "exec_bridge.tools");
  assert.deepEqual(JSON.parse(JSON.stringify(c)), { name: "worker_exec", up: "", tag: "exec_bridge.tools", note: "This agent has no view of its own." });
  // the agent's own reflect wins over the record's
  assert.equal(cardOf("w1", { handler_module: "a.tools" }, "b.tools").tag, "a.tools");
});

test("crumbOf is the names of the path, outermost first, none blank", () => {
  const c2 = vm.createContext({});
  const at = src.indexOf("function crumbOf");
  vm.runInContext(`${src.slice(at, src.indexOf("// end pure", at))}\nthis.crumbOf = crumbOf;`, c2);
  assert.deepEqual(JSON.parse(JSON.stringify(c2.crumbOf([{ name: "lead" }, { name: "" }, { name: "k-rollout" }, null]))), ["lead", "k-rollout"]);
  assert.deepEqual(JSON.parse(JSON.stringify(c2.crumbOf(undefined))), []);
});

test("the title bar's path: the AI agents above the agent, then its name, from the kernel's tree", () => {
  const tree = { id: "kernel_state", display_name: "launch-week", children: [
    { id: "user", children: [{ id: "lead", display_name: "Lead", children: [
      { id: "tasks", children: [] },
      { id: "ops", display_name: "cluster ops", children: [{ id: "k_rollout", display_name: "k-rollout", children: [] }] }] }] }] };
  const chats = { lead: true, ops: true, k_rollout: true };
  const plain = (x) => JSON.parse(JSON.stringify(x));
  assert.deepEqual(plain(pathTo(tree, "k_rollout")).map((n) => n.id), ["user", "lead", "ops", "k_rollout"], "the root is left out");
  assert.deepEqual(plain(crumbPath(pathTo(tree, "k_rollout"), chats)), ["Lead", "cluster ops", "k-rollout"]);
  assert.deepEqual(plain(crumbPath(pathTo(tree, "tasks"), chats)), ["Lead", "tasks"], "an agent that is not an AI agent keeps its own name");
  assert.deepEqual(plain(pathTo(tree, "nope")), []);
  assert.deepEqual(plain(pathTo(null, "lead")), []);
});

test("inquiryEvent is the sidebar's inquiries_changed, never a copy of a call to the sidebar", () => {
  assert.equal(inquiryEvent({ type: "inquiries_changed", open: 2 }), true);
  assert.equal(inquiryEvent({ type: "inquiries_changed", open: 0 }), true);
  // copies of calls made to `sidebar`: the sidebar view's own read, another client's
  assert.equal(inquiryEvent({ type: "reflect" }), false);
  assert.equal(inquiryEvent({ type: "watch" }), false);
  // a call that only borrows the name carries no numeric open
  assert.equal(inquiryEvent({ type: "inquiries_changed" }), false);
  assert.equal(inquiryEvent({ type: "inquiries_changed", open: "2" }), false);
  // the sidebar's other events, a refusal, nothing
  assert.equal(inquiryEvent({ type: "tree_changed" }), false);
  assert.equal(inquiryEvent({ type: "status_changed", agent: "lead", phase: "thinking" }), false);
  assert.equal(inquiryEvent({ error: "denied", reason: "denied" }), false);
  assert.equal(inquiryEvent(null), false);
});

// THE BADGE FOLLOWS THE SIDEBAR AGENT (#351): the page run whole, `sidebar` watched once (never
// `user`), the person's `inquiries` read again on each `inquiries_changed` the sidebar publishes and
// on no copy of a call.
const PAGE = fileURLToPath(new URL("../web/team.html", import.meta.url));
let open = [];
const pageReply = (t, p) => (t === "user" && p.type === "inquiries" ? { inquiries: open } : { error: "no", reason: "denied" });

test("the page watches the sidebar once, sets no timer and reads the inquiries on each published change", async () => {
  open = [];
  const v = runView(PAGE, "", pageReply);
  await v.settle();
  assert.deepEqual(watched(v), [["sidebar", 1]]);
  assert.deepEqual(v.intervals, [], "a page that sets a timer polls");
  open = [{ re: 4, asker: "lead" }, { re: 5, asker: "ops" }];
  let before = v.sends.length;
  await v.publish("sidebar", { type: "inquiries_changed", open: 2 });
  assert.deepEqual(verbsSince(v, before, "user"), ["inquiries"], "one read, and none on its copy");
  const badge = v.document.getElementById("back-badge");
  assert.equal(badge.textContent, "2");
  assert.equal(badge.hidden, false);
  open = [];
  before = v.sends.length;
  await v.publish("sidebar", { type: "inquiries_changed", open: 0 });
  assert.deepEqual(verbsSince(v, before, "user"), ["inquiries"]);
  assert.equal(badge.hidden, true);
});

test("copies of the page's own reads, and another client's calls to the sidebar, read nothing", async () => {
  open = [];
  const v = runView(PAGE, "", pageReply);
  await v.settle();
  await v.publish("sidebar", { type: "inquiries_changed", open: 1 });
  await ownCopiesReadNothing(v);
  assert.equal(await sendsOnOthers(v, [
    ["sidebar", { type: "reflect" }],
    ["sidebar", { type: "watch" }],
    ["sidebar", { type: "tree_changed" }],
    ["sidebar", { error: "the leg may not call sidebar watch", reason: "denied" }],
  ]), 0);
});
