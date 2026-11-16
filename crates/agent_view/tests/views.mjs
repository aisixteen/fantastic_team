// A team agent's html view, run WHOLE on fantastic's view harness (`fantastic_tui::VIEW_HARNESS`,
// in the checkout Cargo pins for `fantastic-tui`: found with `cargo metadata`, no copy): its
// recorded sends and watches, the copy of every call mirrored to the target's watchers as the
// kernel does, its timers, and its text DOM (`textDom`: elements that keep their text, which
// the tests read). The team adds only its template filled the way `team_agent_view::page`
// fills it (`{{helpers}}`, `{{agent}}`, any other placeholder given).
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = fileURLToPath(new URL("../../../", import.meta.url));
const meta = JSON.parse(execFileSync("cargo", ["metadata", "--format-version", "1", "--locked"], {
  cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28,
}));
const tui = meta.packages.find((p) => p.name === "fantastic-tui");
if (!tui) throw new Error("cargo metadata names no fantastic-tui package");
const TUI = dirname(tui.manifest_path);
const AT = /pub const VIEW_HARNESS: &str = "([^"]+)";/.exec(readFileSync(join(TUI, "src", "lib.rs"), "utf8"));
if (!AT) throw new Error("fantastic-tui declares no VIEW_HARNESS");
const base = await import(pathToFileURL(join(TUI, AT[1])).href);
export const { clocksReadNothing, ownCopiesReadNothing } = base;

const HELPERS = readFileSync(fileURLToPath(new URL("../src/helpers.js", import.meta.url)), "utf8");

/**
 * Run the view `file` as agent `agent` on fantastic's harness; `reply(target, payload)` answers
 * each send, `fill` fills more placeholders. Returns the harness (`sends` [{target, p}],
 * `watches`, `intervals`, `settle`, `publish`, `since`) and `document`, the text DOM.
 */
export function runView(file, agent, reply, fill = {}) {
  return base.run(readFileSync(file, "utf8"), { helpers: HELPERS, agent, ...fill }, reply, { textDom: true });
}

/** The verbs (`p.type`) of the sends made to `target` since `mark`. */
export function verbsSince(h, mark, target) {
  return h.since(mark).filter((s) => s.target === target).map((s) => s.p.type);
}

/** An event on `src`'s inbox handed to each of the view's watchers now, without waiting for
 *  what it starts (`publish` waits; this does not, and a source nobody watches is no error). */
export function deliver(h, src, ev) {
  (h.watches.get(src) ?? []).slice().forEach((cb) => cb(JSON.parse(JSON.stringify(ev))));
}

/** The sources the view watches, each with how many watchers it holds on it. */
export function watched(h) {
  return [...h.watches].filter(([, l]) => l.length > 0).map(([src, l]) => [src, l.length]);
}

/** Other clients' calls to the view's agents, mirrored to its watchers: how many sends they cost
 *  (0 when the view keeps the rule). */
export async function sendsOnOthers(h, others) {
  await h.settle();
  const mark = h.sends.length;
  for (const [src, p] of others) deliver(h, src, p);
  await h.settle();
  return h.sends.length - mark;
}
