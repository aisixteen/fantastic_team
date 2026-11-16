// The team page's pure block (waitingCount, cardOf), run in a bare vm context. `node --test crates/team/tests`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { runView, verbsSince, sendsOnCopies } from "../../agent_view/tests/view_harness.mjs";

const src = readFileSync(fileURLToPath(new URL("../web/team.html", import.meta.url)), "utf8");
const from = src.indexOf("// pure\n");
const to = src.indexOf("// end pure", from);
assert.ok(from >= 0 && to > from);
const ctx = vm.createContext({});
vm.runInContext(`${src.slice(from, to)}\nthis.api = { waitingCount, cardOf, inquiryEvent };`, ctx);
const { waitingCount, cardOf, inquiryEvent } = ctx.api;

test("waitingCount counts askers once, other than the open agent", () => {
  const inq = [{ asker: "a" }, { asker: "a" }, { asker: "b" }, { asker: "c" }, { text: "x" }, null];
  assert.equal(waitingCount(inq, "c"), 2);
  assert.equal(waitingCount(inq, "zz"), 3);
  assert.equal(waitingCount(undefined, "a"), 0);
  assert.equal(waitingCount("no", "a"), 0);
});

test("cardOf names the agent, tags its handler, says it has no view", () => {
  const c = cardOf("w1", { display_name: "Worker", handler_module: "worker.tools" });
  assert.deepEqual(JSON.parse(JSON.stringify(c)), { name: "Worker", tag: "worker.tools", note: "This agent has no view of its own." });
  assert.equal(cardOf("w1", {}).name, "w1");
  assert.equal(cardOf("w1", null).tag, "");
});

test("cardOf takes the bundle from the kernel's record when the agent's reflect does not name it", () => {
  // a bridge's reflect carries no handler_module and no display_name (#351)
  const c = cardOf("worker_exec", { id: "worker_exec", sentence: "x" }, "exec_bridge.tools");
  assert.deepEqual(JSON.parse(JSON.stringify(c)), { name: "worker_exec", tag: "exec_bridge.tools", note: "This agent has no view of its own." });
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

test("inquiryEvent is the person's inquiry_opened / inquiry_closed, never a copy of a call to user", () => {
  assert.equal(inquiryEvent({ type: "inquiry_opened", re: 4, asker: "lead", holder: "user" }), true);
  assert.equal(inquiryEvent({ type: "inquiry_closed", re: 4, asker: "lead", holder: "user" }), true);
  // copies of calls made to `user`: the page's own read, another client's answer
  assert.equal(inquiryEvent({ type: "inquiries" }), false);
  assert.equal(inquiryEvent({ type: "answer_inquiry", re: 4, answer: "yes" }), false);
  // a call that only borrows the name carries no holder and no numeric re
  assert.equal(inquiryEvent({ type: "inquiry_opened" }), false);
  assert.equal(inquiryEvent({ type: "inquiry_opened", re: "4", holder: "user" }), false);
  // another holder's inquiry, a refusal, nothing
  assert.equal(inquiryEvent({ type: "inquiry_opened", re: 4, holder: "lead" }), false);
  assert.equal(inquiryEvent({ error: "denied", reason: "denied" }), false);
  assert.equal(inquiryEvent(null), false);
});

// THE BADGE FOLLOWS THE PERSON'S INQUIRIES (#351): the page run whole, `user` watched once, its
// `inquiries` read again on each published inquiry_opened / inquiry_closed and on no copy of a call.
const PAGE = fileURLToPath(new URL("../web/team.html", import.meta.url));
let open = [];
const pageReply = (t, p) => (t === "user" && p.type === "inquiries" ? { inquiries: open } : { error: "no", reason: "denied" });

test("the page watches user once, sets no timer and reads its inquiries on each published change", async () => {
  open = [];
  const v = runView(PAGE, "", pageReply, {}, { mirror: true });
  await v.settle();
  assert.deepEqual(v.watches.map((w) => w.src), ["user"]);
  assert.deepEqual(v.timers, [], "a page that sets a timer polls");
  open = [{ re: 4, asker: "lead" }, { re: 5, asker: "ops" }];
  let before = v.sends.length;
  v.deliver("user", { type: "inquiry_opened", re: 5, asker: "ops", holder: "user" });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, "user"), ["inquiries"], "one read, and none on its copy");
  const badge = v.document.getElementById("back-badge");
  assert.equal(badge.textContent, "2");
  assert.equal(badge.hidden, false);
  open = [];
  before = v.sends.length;
  v.deliver("user", { type: "inquiry_closed", re: 5, asker: "ops", holder: "user" });
  await v.settle();
  assert.deepEqual(verbsSince(v, before, "user"), ["inquiries"]);
  assert.equal(badge.hidden, true);
});

test("copies of the page's own inquiries read, and another client's reads and answers, read nothing", async () => {
  open = [];
  const v = runView(PAGE, "", pageReply, {}, { mirror: true });
  await v.settle();
  v.deliver("user", { type: "inquiry_opened", re: 1, asker: "lead", holder: "user" });
  await v.settle();
  assert.equal(await sendsOnCopies(v, [
    ["user", { type: "inquiries" }],
    ["user", { type: "status" }],
    ["user", { type: "answer_inquiry", re: 1, answer: "yes" }],
    ["user", { error: "the leg may not call user watch", reason: "denied" }],
  ]), 0);
});
