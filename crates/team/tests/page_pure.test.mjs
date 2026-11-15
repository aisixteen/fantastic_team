// The team page's pure block (waitingCount, cardOf), run in a bare vm context. `node --test crates/team/tests`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const src = readFileSync(fileURLToPath(new URL("../web/team.html", import.meta.url)), "utf8");
const from = src.indexOf("// pure\n");
const to = src.indexOf("// end pure", from);
assert.ok(from >= 0 && to > from);
const ctx = vm.createContext({});
vm.runInContext(`${src.slice(from, to)}\nthis.api = { waitingCount, cardOf };`, ctx);
const { waitingCount, cardOf } = ctx.api;

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
