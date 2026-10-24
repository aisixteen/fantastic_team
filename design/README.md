# Design briefs

This folder carves one app's screens into briefs for Claude Design. `scripts/design-sync.sh` writes it from a run of the `design-briefs` task. The loop:

1. **Brief.** `NN-screen.md` says what the screen is for, what data it shows, its states and its actions.
2. **Claude Design.** Give the brief to Claude Design and get the screen as HTML.
3. **html/.** Save the page as `html/NN-screen.html`. Only the maintainer writes here. The task never creates, changes or deletes anything in `html/`.
4. **UI.** Map the pasted page into the app's UI.

`manifest.md` lists each screen with its brief hash, its status and whether its html exists. When a brief changes, its status reads "brief changed, html older than brief" until a newer html file is pasted over the old one.
