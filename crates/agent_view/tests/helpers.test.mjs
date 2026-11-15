// The shared view helpers' pure block and the pure block of each view that uses them,
// run in a bare vm context (no DOM). `node --test crates/agent_view/tests`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const at = (rel) => fileURLToPath(new URL(rel, import.meta.url));
const pureOf = (file, mark) => {
  const src = readFileSync(file, "utf8");
  const from = src.indexOf(mark);
  const to = src.indexOf("// end pure", from);
  assert.ok(from >= 0 && to > from, `${file} marks ${mark}`);
  return src.slice(from, to);
};
const plain = (x) => JSON.parse(JSON.stringify(x));

function load(file, mark, names) {
  const ctx = vm.createContext({});
  const shared = pureOf(at("../src/helpers.js"), "// pure\n");
  const own = file ? pureOf(file, mark) : "";
  vm.runInContext(`${shared}\n${own}\nthis.api = { ${names.join(", ")} };`, ctx);
  return ctx.api;
}

const shared = load(null, "", ["txt", "count", "secText", "bytesText", "resultClass", "barParts"]);

test("count is a whole non-negative number, else 0", () => {
  assert.equal(shared.count(5), 5);
  assert.equal(shared.count(2.9), 2);
  assert.equal(shared.count(-1), 0);
  assert.equal(shared.count("7"), 0);
  assert.equal(shared.count(undefined), 0);
});

test("secText and bytesText", () => {
  assert.equal(shared.secText(60), "60 s");
  assert.equal(shared.secText(null), "-");
  assert.equal(shared.bytesText(512), "512 B");
  assert.equal(shared.bytesText(5325), "5.2 KB");
  assert.equal(shared.bytesText(undefined), "-");
});

test("a result word has its class: ok, refused amber, unreachable and errors red", () => {
  assert.equal(shared.resultClass("ok"), "ok");
  assert.equal(shared.resultClass("refused"), "warn");
  assert.equal(shared.resultClass("error 404"), "bad");
  assert.equal(shared.resultClass("unreachable"), "bad");
  assert.equal(shared.resultClass("weird"), "");
});

test("a bar keeps only the non-zero parts", () => {
  const parts = [{ key: "a", n: 3 }, { key: "b", n: 0 }, { key: "c", n: 1 }];
  assert.deepEqual(plain(shared.barParts(parts)).map((p) => p.key), ["a", "c"]);
  assert.deepEqual(plain(shared.barParts(undefined)), []);
});

const cad = load(at("../../team/src/cadencer/view.html"), "// pure (cadencer)", ["loadParts", "settingsOf", "wholeSeconds"]);

test("cadencer load: a stalled worker is counted once, not as busy too", () => {
  const p = plain(cad.loadParts({ busy: 6, idle: 2, stalled: 1 }));
  assert.deepEqual(p.map((x) => [x.key, x.n]), [["busy", 5], ["idle", 2], ["stalled", 1]]);
  assert.deepEqual(plain(cad.loadParts(undefined)).map((x) => x.n), [0, 0, 0]);
  assert.equal(cad.loadParts({ busy: 0, stalled: 2 })[0].n, 0, "never negative");
});

test("cadencer settings come out whole whatever the status holds", () => {
  const s = plain(cad.settingsOf({ settings: { enabled: true, cadence: 60, stall_after: 300, quiet: false, off: ["m-ads", 3, ""] } }));
  assert.deepEqual(s, { enabled: true, quiet: false, cadence: 60, stall_after: 300, off: ["m-ads"] });
  assert.deepEqual(plain(cad.settingsOf(null)), { enabled: false, quiet: false, cadence: null, stall_after: null, off: [] });
});

test("cadencer seconds fields take positive whole numbers only", () => {
  assert.equal(cad.wholeSeconds("90"), 90);
  assert.equal(cad.wholeSeconds(" 5 "), 5);
  for (const bad of ["", "0", "-3", "1.5", "abc"]) assert.equal(cad.wholeSeconds(bad), null, bad);
});

const gh = load(at("../../github_connector/src/view.html"), "// pure (github)", ["authChip", "newestFirst", "checksText", "reviewText", "shortSha", "prNumber"]);

test("github auth chip: a usable credential is ok, local amber, invalid red", () => {
  assert.deepEqual(plain(gh.authChip({ mode: "app" })), { text: "auth: app", cls: "ok" });
  assert.deepEqual(plain(gh.authChip({ mode: "personal" })), { text: "auth: personal", cls: "ok" });
  assert.equal(gh.authChip({ mode: "local" }).cls, "warn");
  assert.equal(gh.authChip({ mode: "invalid" }).cls, "bad");
  assert.deepEqual(plain(gh.authChip(undefined)), { text: "auth: unknown", cls: "" });
});

test("github recent calls are shown newest first, without touching the reply", () => {
  const calls = [{ verb: "a" }, null, { verb: "b" }, { verb: "c" }];
  assert.deepEqual(plain(gh.newestFirst(calls)).map((c) => c.verb), ["c", "b", "a"]);
  assert.equal(calls[0].verb, "a");
  assert.deepEqual(plain(gh.newestFirst(undefined)), []);
});

test("github checks read as passed, pending and failed", () => {
  const run = (status, conclusion) => ({ status, conclusion });
  const five = Array.from({ length: 5 }, () => run("completed", "success"));
  assert.equal(gh.checksText([...five, run("in_progress", null)]), "5 passed, 1 pending");
  assert.equal(gh.checksText([run("completed", "failure"), run("completed", "skipped")]), "1 passed, 1 failed");
  assert.equal(gh.checksText([]), "none");
  assert.equal(gh.checksText(undefined), "none");
});

test("github review text, short sha and the number field", () => {
  assert.equal(gh.reviewText("changes_requested"), "changes requested");
  assert.equal(gh.reviewText(undefined), "-");
  assert.equal(gh.shortSha("9f3c2abdeadbeef"), "9f3c2ab");
  assert.equal(gh.shortSha(undefined), "-");
  assert.equal(gh.prNumber("703"), 703);
  for (const bad of ["", "0", "x", "1.5"]) assert.equal(gh.prNumber(bad), null, bad);
});

const res = load(at("../../agentic_resources/src/view.html"), "// pure (agentic_resources)", ["kindRows", "runText", "errorsText", "errorLine", "grantNames", "chosen"]);

test("resources: a kind row counts the agents the fleet lists for it", () => {
  const rows = plain(res.kindRows(
    [{ name: "worker", purpose: "Does one task" }, { name: "reviewer", purpose: "Reads a diff" }, { purpose: "no name" }],
    { bundles: { worker: [{ id: "w1" }, { id: "w2" }, {}] } }
  ));
  assert.deepEqual(rows.map((r) => [r.name, r.running]), [["worker", 3], ["reviewer", 0]]);
  assert.deepEqual(rows[0].ids, ["w1", "w2"]);
  assert.deepEqual(plain(res.kindRows(undefined, undefined)), []);
});

test("resources: the chip, the errors line, the grants and the chosen kind", () => {
  assert.equal(res.runText(7), "7 running");
  assert.equal(res.runText(undefined), "0 running");
  assert.equal(res.errorsText([]), "");
  assert.equal(res.errorsText(["x"]), "1 error");
  assert.equal(res.errorsText(["x", "y"]), "2 errors");
  assert.equal(res.errorLine("bundles/old_scout: no directive"), "bundles/old_scout: no directive");
  assert.equal(res.errorLine({ error: "boom" }), "boom");
  assert.deepEqual(plain(res.grantNames({ team_github: ["x"], task_registry: ["y"] })), ["team_github", "task_registry"]);
  assert.deepEqual(plain(res.grantNames(null)), []);
  const rows = [{ name: "a" }, { name: "b" }];
  assert.equal(res.chosen(rows, "b").name, "b");
  assert.equal(res.chosen(rows, "gone").name, "a");
  assert.equal(res.chosen([], "a"), null);
});
