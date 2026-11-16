# ui: the team page

Two kinds of test live here.

- The older ones (`sidebar_renders`, `chat_in_right_cell`, `layout_two_columns`, `worker_view_opens`, `sidebar_opens_agent_full_screen`) each prove one piece.
- The functional ones (#351) use the page like a person: `main_opens_each_agent`, `tasks_view_counts_and_detail`, `tool_views_real_data`, `file_row_opens_file`, `phone_one_panel`, `chat_send_and_answer`, `agent_states_in_tree` on the fleet `fleets/team_ui`, and `seeded_page_opens_agents` on the product's own seed. `helpers/team_ui.py` holds the fleet's data, the seeding and the browser.

They test FUNCTION: a click opens the right agent, the view shows that agent's real data (exact names, counts and texts of the fleet), a message or an answer arrives. They do not test the design: the look, the layout and the wording of labels are a separate task (the design mock in `design/mock/`).

All are free (scripted model, no key) and need Playwright with Chrome: `python3 -m pip install --user playwright==1.55.0`. Without it a probe fails with a message; it never skips.

```
python3 e2e/run.py --only ui/main_opens_each_agent --bin /ABSOLUTE/path/fantastic_team
```

## Live, never polled

An agent is the model; each view is a view-model that watches its own agent and re-reads when the agent publishes a change (`task_changed`, `settings_changed` / `ticked`, `call_served`, `fleet_changed`). `tasks_view_counts_and_detail` and `tool_views_real_data` change the agent outside the page (an operator call) and check the open view shows it within `team_ui.LIVE_S` (3 s) with no reload, and `team_ui.view_follows` fails a view whose served source sets a timer or whose calls do not name `watch`.

## The fleet's grants

The page's leg `team_ws` holds, per agent, the calls of that agent's view (`fleets/team_ui/agents/web/agents/team_ws/agent.yaml`), declared in the records on both ends. `seeded_page_opens_agents` declares nothing: it shows what the product's seed lets the page open.

## Known gaps: not tested, no data source yet

These need a source before a test can ask for them:

- stalled per agent (the `!` mark);
- paused per agent (not in `status`);
- the last outcome of an agent in the tree (failed, done);
- a task's title (the registry has none);
- the names of other kernels and what waits in them;
- the health dot on a tool agent;
- an "in review" task state (not a registry state).
