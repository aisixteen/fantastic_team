<!-- template: design-briefs
field APP
field REPO
field SRC
field COMMIT=main
field BRANCH
field BASE=main
field CHECK=sh scripts/design-sync.sh --check design
field GIT=git
field GITHUB=github
-->
# {{TITLE}}

_Task kind: design-briefs · written {{DATE}} by {{AUTHOR}}._ Carve the screens of one app into Markdown briefs under `design/` in the app's repository, one brief per screen, for Claude Design to turn into HTML. Deliver the briefs as a pushed branch and a draft pull request. A run that changes nothing opens no pull request.

## Rules — read first
- Write only under `design/` in WT, and only through the staging directory and `scripts/design-sync.sh`. Never write, create, move or delete anything under `design/html/`: pasted pages belong to the maintainer.
- Change no code. The pull request holds `design/` and nothing else.
- Write every brief from scratch from the source at `{{COMMIT}}`. Never copy an old brief forward and edit it.
- No timestamps, dates, random values or machine paths inside a brief. Sort every list. The same source gives the same brief.
- No placeholder text. Sample rows look real: plausible names, counts and states.
- No auth, accounts, sign-in, billing or SaaS. No provider or model names in a layout; they are data, shown in a row if at all.
- A brief shows data and sends verbs. It holds no logic beyond that.
- Never push to `{{BASE}}`, never force-push, never merge.
- Write `For #N` in pull request text. Never write "close", "fix" or "resolve" next to an issue number.
- The same failure after two honest fix attempts: stop, and hand the task back as failed with the step, the error and your diagnosis.

## The cycle (do all of it, in order)
1. **Worktree.** Ask the `{{GIT}}` connector for a worktree on a new branch `{{BRANCH}}` from `{{COMMIT}}` (verb `worktree_add`). Work only inside the path it returns (WT). Read `design/manifest.md` in WT if it exists: it holds the screen numbers and names already in use.
2. **Read** `{{SRC}}` in `{{REPO}}` at `{{COMMIT}}`, and the issues the task names, so each brief matches what the code does. List the screens: one view, panel, dialog or layout a user sees.
3. **Name** each screen `<NN>-<screen>`, lowercase words joined by `-`. A screen that `design/manifest.md` already lists keeps its number and name. A new screen takes the next free number. A number is never reused, not even for a removed screen.
4. **Write** every brief from scratch into `WT/.design-staging/`: `00-overview.md` first, then one `<NN>-<screen>.md` per screen. Each brief has these level-2 headings, exactly, in this order: `## Purpose and audience`, `## Data shown`, `## States`, `## Actions`, `## Shared rules`.
   - Purpose and audience: one or two sentences, and who looks at the screen.
   - Data shown: each item named after the verb or endpoint that provides it, with realistic sample rows.
   - States: loading, empty, error, many items, and a stalled or failed row.
   - Actions: each action, and where it leads.
   - Shared rules: the rules from `00-overview.md`, restated: light and dark themes, phone and desktop widths, and the same page in a browser and in the app.
5. **Sync.** In WT run `sh scripts/design-sync.sh --app {{APP}} --commit <sha> --staging WT/.design-staging --out WT/design`, where `<sha>` is the commit from step 1. Keep its stdout, one line per change or "no change". Then delete `WT/.design-staging/`.
6. **Check.** In WT run `{{CHECK}}`. It must print `design: ok`.
7. **Commit and deliver.** If `git status` shows no change under `design/`, report "no change", open no pull request, and go to step 8. Otherwise commit with the `{{GIT}}` connector on `{{BRANCH}}`, message `design: briefs for {{APP}} at <short sha>`; push `{{BRANCH}}`; and open a draft pull request with that title, against `{{BASE}}`, through the `{{GITHUB}}` connector. Its body lists the changed screens, one line each, from the script's stdout, and ends with `For #96.`
8. **Clean up.** Remove the worktree (verb `worktree_remove`).
9. **Report** in one short paragraph: the branch, the commit, the pull request link or "no change", and the screens changed.

## The task
<!-- WRITE: who reads these briefs and what Claude Design must be able to build from each one. -->

**Screens to carve:** <!-- WRITE: the screens for this app and their sources. For the first run on this repo (APP=fantastic_team, SRC=crates/team/web), the list is below; replace it for another app. -->

- `00-overview`: the shared rules every brief restates. Source: `crates/team/web/dispatcher.html`, `crates/team/web/worker_view.css`.
- `01-dispatcher` (#58): the dispatcher page with its tree, worker rows, connections and waiting list. Source: `crates/team/web/dispatcher.html`, `crates/team/src/dispatcher.rs`, `crates/team/src/tree.rs`.
- `02-group-canvas`: one group as a canvas of its workers. Source: `crates/team/src/tree.rs`, `crates/team/src/board.rs`.
- `03-worker-canvas` (#36): one worker's view. Source: `crates/team/web/worker_view.css`, `crates/team/src/worker_view.rs`.
- `04-escalations` (#95): the escalations list and its actions. Source: `crates/team/src/dispatcher.rs`, `crates/team/src/board.rs`.
- `05-connections`: the connections panel. Source: `crates/team/web/dispatcher.html`, `crates/team/src/dispatcher.rs`.
- `06-settings`: the settings dialog. Source: `crates/team/web/dispatcher.html`.
- `07-phone`: the phone layout of the screens above. Source: `crates/team/web/worker_view.css`.

**Issues the briefs must agree with:** #58, #95, #36, #37. <!-- WRITE: add or remove issues. -->

**Commit message:** `design: briefs for {{APP}} at <short sha>` (step 7).
