# ui/main_opens_each_agent (free)

The main page, used like a person uses it (#351). Function only: no check is about the look or a label's wording.

Fleet: `fleets/team_ui` (record files; `helpers/team_ui.py` says what is seeded with the team's own verbs). Chrome (Playwright) opens the real page of the real binary, scripted model, no key.

- the title bar names the team and its kernel; the tree lists the six AI agents by name, nested as the fleet is, and the tool agents;
- "Waiting for you" lists the two real questions (asker, text, age), and a click on a row opens that asker;
- a click on each kind of row opens THAT agent's view with its data: an AI agent (its own name, its own messages, its own question and options), the task registry (the seeded tasks counted), the cadencer (its record's switch and switched-off lead), the GitHub connector (its auth mode and repo), the bundles registry (its kinds), an agent with no view (the name card);
- the filter keeps the matching agent and its branch.
