// THE TASK REGISTRY VIEW'S PURE DATA SHAPING (team#351): the counts by state, the holders,
// the grouped table rows with the done fold, the filters, the history and the last report
// as texts. The view (`src/view.html`) keeps it in one `// pure` script block: it is read
// here and run in a bare vm context, no DOM. Run: `node --test crates/task_registry/tests`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const HTML = fileURLToPath(new URL("../src/view.html", import.meta.url));
const src = readFileSync(HTML, "utf8");
const from = src.indexOf("// pure\n");
const to = src.indexOf("// end pure");
assert.ok(from >= 0 && to > from, "the view marks its pure block");

const ctx = vm.createContext({});
vm.runInContext(
  `${src.slice(from, to)}\nthis.api = { STATES, GROUP_ORDER, LEGEND_ORDER, DONE_SHOWN, stateOf, holderText, holderKey, taskList, countsOf, holdersOf,
  subline, kpiTasks, kpiHolders, legendRows, filterTasks, holderOptions, groupedRows, clockText, historyRows, reportOf, versionRows, errorText };`,
  ctx,
);
const api = ctx.api;
// the vm's objects are of another realm: compare them as plain JSON
const plain = (x) => JSON.parse(JSON.stringify(x));

const task = (id, status, holder, extra = {}) => Object.assign({ id, source: "gh:o/r#" + id, status, holder }, extra);
const TASKS = [
  task("1", "running", "w1"),
  task("2", "running", "w2"),
  task("3", "waiting", null),
  task("4", "blocked", "lead"),
  task("5", "failed", "w1"),
  task("6", "paused", { subtree: "sub" }),
  task("7", "done", "w1"),
  task("8", "done", "w2"),
  task("9", "done", "w1"),
  task("10", "done", "w2"),
  task("11", "done", "w1"),
];

test("the six real states are the states; an unknown or missing status reads as waiting", () => {
  assert.deepEqual(plain(api.STATES), ["waiting", "running", "blocked", "done", "failed", "paused"]);
  assert.equal(api.stateOf({ status: "paused" }), "paused");
  assert.equal(api.stateOf({ status: "in review" }), "waiting");
  assert.equal(api.stateOf({}), "waiting");
});

test("counts per state, the total, and held = running (the board's rule)", () => {
  const c = plain(api.countsOf(TASKS));
  assert.deepEqual(c, { total: 11, held: 2, waiting: 1, running: 2, blocked: 1, done: 5, failed: 1, paused: 1 });
  assert.equal(api.subline(c), "11 tasks · 2 held · 1 blocked");
  assert.equal(api.kpiTasks(c), "2 held · 1 waiting · 1 blocked");
  assert.equal(api.subline(api.countsOf([task("1", "running", "a")])), "1 task · 1 held · 0 blocked");
});

test("a holder is an agent id, a subtree or nobody; distinct holders are counted by kind", () => {
  assert.equal(api.holderText("w1"), "w1");
  assert.equal(api.holderText({ subtree: "sub" }), "subtree sub");
  assert.equal(api.holderText(null), "nobody");
  assert.equal(api.holderText(""), "nobody");
  assert.equal(api.holderKey("w1"), "a:w1");
  assert.equal(api.holderKey({ subtree: "sub" }), "s:sub");
  assert.equal(api.holderKey(null), "");
  const h = plain(api.holdersOf(TASKS));
  assert.deepEqual(h, { total: 4, agents: 3, subtrees: 1 });
  assert.equal(api.kpiHolders(h), "3 agents · 1 subtree");
  assert.equal(api.kpiHolders({ agents: 1, subtrees: 0 }), "1 agent · 0 subtrees");
});

test("the status bar lists the states that have tasks, in the mock's order, each with a skin token (the shared bar's parts)", () => {
  const rows = plain(api.legendRows(api.countsOf(TASKS)));
  assert.deepEqual(rows.map((r) => [r.label, r.n]), [["done", 5], ["running", 2], ["blocked", 1], ["failed", 1], ["paused", 1], ["waiting", 1]]);
  for (const r of rows) {
    assert.match(r.color, /^var\(--fc-color-/);
    assert.equal(r.key, r.label);
  }
  assert.deepEqual(plain(api.legendRows(api.countsOf([task("1", "failed", "a")]))).map((r) => r.label), ["failed"]);
});

test("the table groups by state in the mock's order and shows failed and paused; done folds past three", () => {
  const rows = plain(api.groupedRows(TASKS, false));
  const heads = rows.filter((r) => r.kind === "head").map((r) => [r.label, r.n]);
  assert.deepEqual(heads, [["Blocked", 1], ["Failed", 1], ["Running", 2], ["Paused", 1], ["Waiting", 1], ["Done", 5]]);
  const doneItems = rows.filter((r) => r.kind === "item" && r.state === "done");
  assert.equal(doneItems.length, api.DONE_SHOWN);
  assert.deepEqual(rows[rows.length - 1], { kind: "more", n: 2 });
  const paused = rows.find((r) => r.kind === "item" && r.state === "paused");
  assert.deepEqual(paused, { kind: "item", id: "6", state: "paused", holder: "subtree sub", nobody: false });
  const waiting = rows.find((r) => r.kind === "item" && r.state === "waiting");
  assert.equal(waiting.holder, "nobody");
  assert.equal(waiting.nobody, true);
  const open = plain(api.groupedRows(TASKS, true));
  assert.equal(open.filter((r) => r.kind === "item" && r.state === "done").length, 5);
  assert.ok(!open.some((r) => r.kind === "more"));
  assert.deepEqual(plain(api.groupedRows([], false)), []);
});

test("the filters are the list verb's own: a state, a holder key, or any", () => {
  assert.deepEqual(plain(api.filterTasks(TASKS, "", "")).length, 11);
  assert.deepEqual(plain(api.filterTasks(TASKS, "done", "")).map((t) => t.id), ["7", "8", "9", "10", "11"]);
  assert.deepEqual(plain(api.filterTasks(TASKS, "", "a:w2")).map((t) => t.id), ["2", "8", "10"]);
  assert.deepEqual(plain(api.filterTasks(TASKS, "done", "a:w2")).map((t) => t.id), ["8", "10"]);
  assert.deepEqual(plain(api.filterTasks(TASKS, "", "s:sub")).map((t) => t.id), ["6"]);
  assert.deepEqual(plain(api.holderOptions(TASKS)), [
    { key: "a:lead", text: "lead" }, { key: "s:sub", text: "subtree sub" }, { key: "a:w1", text: "w1" }, { key: "a:w2", text: "w2" },
  ]);
});

test("a list reply's tasks are the well-formed entries; a refusal is its error text", () => {
  assert.deepEqual(plain(api.taskList({ tasks: [task("1", "running", "a"), null, { status: "done" }, "x"] })).map((t) => t.id), ["1"]);
  assert.deepEqual(plain(api.taskList({ error: "x" })), []);
  assert.equal(api.errorText({ error: "task_registry is disabled: the record has no task_store_id", reason: "disabled" }), "task_registry is disabled: the record has no task_store_id");
  assert.equal(api.errorText({ tasks: [] }), "");
  assert.equal(api.errorText(null), "no answer");
});

test("a time is HH:MM on the viewer's day and dated before it", () => {
  const now = Math.floor(Date.now() / 1000);
  const d = new Date(now * 1000);
  const hm = String(d.getHours()).padStart(2, "0") + ":" + String(d.getMinutes()).padStart(2, "0");
  assert.equal(api.clockText(now, now), hm);
  const old = now - 3 * 86400;
  const o = new Date(old * 1000);
  const date = `${o.getFullYear()}-${String(o.getMonth() + 1).padStart(2, "0")}-${String(o.getDate()).padStart(2, "0")}`;
  assert.ok(api.clockText(old, now).startsWith(date + " "), api.clockText(old, now));
  assert.equal(api.clockText(null, now), "");
  assert.equal(api.clockText("1", now), "");
});

test("the history is each step as texts: from, to, by, at", () => {
  const now = 1_760_000_000;
  const entry = { history: [
    { from: null, to: "lead", by: "main", at: now - 60 },
    { from: "lead", to: { subtree: "sub" }, by: null, at: null },
    "junk",
  ] };
  const rows = plain(api.historyRows(entry, now));
  assert.equal(rows.length, 2);
  assert.deepEqual([rows[0].from, rows[0].to, rows[0].by], ["nobody", "lead", "main"]);
  assert.equal(rows[0].at, api.clockText(now - 60, now));
  assert.deepEqual(rows[1], { from: "lead", to: "subtree sub", by: "", at: "" });
  assert.deepEqual(plain(api.historyRows({}, now)), []);
});

test("the last report: its caption names who, when and the state; its result is text or JSON", () => {
  const now = 1_760_000_000;
  const at = now - 120;
  const r = plain(api.reportOf({ report: { from: "w1", state: "running", result: "tests pass", at } }, now));
  assert.equal(r.caption, `Last report from w1, ${api.clockText(at, now)}, state: running`);
  assert.equal(r.text, "tests pass");
  const j = plain(api.reportOf({ report: { from: null, state: "reviewing", result: { ok: true }, at: null } }, now));
  assert.equal(j.caption, "Last report from nobody");
  assert.equal(j.text, JSON.stringify({ ok: true }, null, 2));
  assert.equal(api.reportOf({ report: null }, now), null);
  assert.equal(api.reportOf({}, now), null);
});

test("the versions are the well-formed rows, the commit cut short", () => {
  const rows = plain(api.versionRows({ versions: [{ repo: "o/r", branch: "fix/1", commit: "0123456789abcdef" }, null] }));
  assert.deepEqual(rows, [{ repo: "o/r", branch: "fix/1", commit: "0123456789ab" }]);
  assert.deepEqual(plain(api.versionRows({})), []);
});
