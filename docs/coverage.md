# Test coverage of the closed issues

Direction from the maintainer (2026-10-25, issue #147): the behaviour of every closed issue is covered by tests, units and e2e, so a later change, rewrite or conflict merge cannot silently undo it. The README video lost in #82 (#146) is the example: nothing tested it, so nothing caught it.

This file is the map. `crates/team/tests/coverage_doc.rs` keeps it honest: it checks the table header, that every status is a known word, that every `missing` row has an open line under "Gaps still open", and that every unit test and e2e folder the table names still exists.

## How to update it

When an issue closes, add one row, sorted by issue number, in the same PR that closes it.

- Unit tests go in the unit column as `path/to/file.rs::test_name`, one per backtick span. A shell test is `tests/file.sh::case_name`, the name given to its `ok` call.
- An e2e test goes in the e2e column as `e2e/tests/area/name`. If the behaviour is user-visible or runs in the binary and has none yet, write "deferred to the e2e phase". Write "n/a" with a reason only for docs, design, build config or scripts that a shell test runs for real.
- Only cite a test you have read and that exercises the behaviour.
- A `missing` row also needs a `- [ ] #N` line under "Gaps still open". Tick it (`- [x]`) only when the test it names exists on main.

Status words:

| status | meaning |
| --- | --- |
| covered | unit tests and an e2e (or an n/a with a reason) |
| units only | unit tests; the e2e is deferred to the e2e phase |
| e2e only | an e2e but no unit test of its own; a gap for kernel, routing, exec or secrets code |
| missing | no test found on main |
| n/a | design or superseded; nothing to test |

## Closed issues

The inventory is `gh issue list --state closed` on aisixteen/fantastic_team (59 issues at 2026-10-27).

| issue | behaviour | unit tests | e2e | status |
| --- | --- | --- | --- | --- |
| #2 | README is a guide a third party can follow | `crates/team/tests/readme_guide.rs::readme_has_the_guide_sections_in_order`, `crates/team/tests/readme_guide.rs::every_script_the_readme_names_exists`, `crates/team/tests/readme_guide.rs::readme_examples_hold_no_secret_values` | n/a (docs or design) | covered |
| #3 | No secret ever leaves an agent (the connectors' own secret tests) | `crates/github_connector/src/tests_secrets.rs::the_key_the_jwt_and_the_token_never_leave_the_agent`, `crates/github_connector/src/tests_secrets.rs::a_personal_token_never_leaves_the_agent`, `crates/git_connector/src/tests_secrets.rs::the_https_token_never_leaves_the_agent` | `e2e/tests/secrets/worker_never_sees_tokens` | covered |
| #4 | Headless fantastic host plus this repo's bundles | none | n/a (base behaviour, base tests it) | n/a |
| #5 | git connector: worktrees, branches, commits, push; credentials by ref | `crates/git_connector/src/tests.rs::worktree_add_makes_a_branch_from_base`, `crates/git_connector/src/tests.rs::push_never_forces`, `crates/git_connector/src/tests_secrets.rs::the_https_token_never_leaves_the_agent` | `e2e/tests/task/one_issue_to_draft_pr` | covered |
| #6 | github connector is an agent record; token minted per call | `crates/github_connector/src/tests.rs::a_token_is_minted_per_call_for_the_repo`, `crates/github_connector/src/tests.rs::pr_create_is_a_draft_on_the_default_branch`, `crates/github_connector/src/tests.rs::repos_off_the_allowlist_are_refused_before_any_request` | `e2e/tests/task/github_pr_through_connector` | covered |
| #7 | Queue agent: tasks as data, leased take/renew/done (the queue bundle is fantastic's; only lease behaviour of peers has units here) | none (the peer transport went in #292; the queue is fantastic's) | `e2e/tests/task/one_issue_to_draft_pr` | e2e only |
| #8 | Worker exec: no forge credentials | `crates/team/tests/worker_exec.rs::a_push_from_a_worker_sends_no_credentials`, `crates/team/tests/worker_exec.rs::gh_version_is_not_refused` | n/a | covered |
| #9 | config is agent records: the team's fixed agents are written once from `crates/team/blueprint.yaml` at the first run (#261) and rehydrated from disk after, so the cards' grants are true as written | `crates/team/src/new_team.rs::roles_mode_the_lead_holds_the_registry_and_its_spawn_grants`, `crates/team/src/hydrate.rs::every_card_grant_on_a_tool_names_an_agent_the_blueprint_declares`, `crates/team/src/hydrate.rs::the_first_run_seeds_the_fixed_ids_with_their_grants_and_config_from_the_yaml` | `e2e/tests/task/one_issue_to_draft_pr` | covered |
| #10 | PR footer: lane, model, time, tokens, cost | `crates/github_connector/src/footer.rs::footer_from_fixed_usage_record`, `crates/github_connector/src/footer.rs::pr_create_appends_footer_when_usage_given`, `crates/github_connector/src/footer.rs::footer_contains_no_secret_like_values` | deferred to the e2e phase | units only |
| #11 | Task templates per kind and the generator | `tests/new_task.sh::every_kind_is_listed`, `tests/new_task.sh::fields_and_defaults_are_filled`, `tests/new_task.sh::no_template_holds_a_secret_like_string` | n/a (shell script; the shell test runs it for real) | covered |
| #12 | How-it-works animation (manim to mp4); the file was later deleted by #152 | none | n/a (superseded by #152) | n/a |
| #14 | Directives as role bundles: worker and lead directives load | `crates/team/src/roles.rs::the_role_bundles_load_as_the_registrys_roles`, `crates/team/src/roles.rs::cards_parse_and_have_the_five_coder_names_plus_project_lead` | `e2e/tests/roles/project_lead_plans_and_runs_issues` | covered |
| #15 | Weak binding to fantastic: one pinned commit in Cargo.lock | `crates/team/tests/fantastic_pin.rs::lock_pins_one_fantastic_commit`, `crates/team/tests/fantastic_pin.rs::notes_name_pinned_commit` | n/a (build config) | covered |
| #16 | The first agent's instructions flags are gone: a directive is the record's `system_prompt_file` or a `send`; the flags are refused as fantastic refuses unknown words | `crates/team/src/main.rs::ai_readme_names_fantastic_s_command_line_and_the_removed_flags_only_as_removed` | deferred to the e2e phase | units only |
| #22 | Team tree is agent records (the blueprint's, seeded at the first run): the lead, its registries and connectors | `crates/team/src/new_team.rs::roles_mode_the_lead_holds_the_registry_and_its_spawn_grants`, `crates/team/src/hydrate.rs::every_declared_module_is_registered` | `e2e/tests/task/one_issue_to_draft_pr` | covered |
| #25 | Puller replaces the dispatcher: labelled issues become task files | `crates/team/src/puller.rs::accepts_issue_labelled_by_maintainer_with_maintainer_spec`, `crates/team/src/puller.rs::rejects_label_added_by_non_maintainer`, `crates/team/src/puller.rs::pull_does_not_overwrite_existing_state`, `crates/github_connector/src/tests.rs::label_events_returns_only_label_events_with_actors` | `e2e/tests/task/one_issue_to_draft_pr` | covered |
| #28 | Issue-keeper role: edit one issue line, set a board field, never touch the SPEC | `crates/github_connector/src/tests.rs::issue_edit_replaces_exactly_the_one_matching_line`, `crates/github_connector/src/tests.rs::issue_edit_refuses_a_line_inside_the_spec_section`, `crates/github_connector/src/tests.rs::project_field_set_sends_one_graphql_mutation_with_item_field_value`, `crates/github_connector/src/verbs.rs::spec_section_is_detected_until_next_heading` | deferred to the e2e phase | units only |
| #29 | Team board: pulls open issues, tracks claims, audits, asks, never assigns | `crates/team/src/board.rs::pull_caches_all_open_issues_from_mock_github`, `crates/team/src/board.rs::audit_never_assigns`, `crates/team/src/board.rs::audit_flags_double_claim`, `crates/team/src/board.rs::ask_is_recorded_with_answer` | `e2e/tests/dispatcher/digest_reaches_driver` | covered |
| #30 | App registration for third parties: manifest flow; removed by #303 | none | n/a (removed by #303: auth is configuration, env refs) | n/a |
| #31 | Per-repo configuration: the `repo_config` field of the puller's record | `crates/team/src/repo_config.rs::parses_full_config`, `crates/team/src/repo_config.rs::rejects_unknown_key`, `crates/team/src/repo_config.rs::rejects_parent_dir_in_paths`, `crates/team/src/puller.rs::invalid_repo_config_skips_repo_with_error` | deferred to the e2e phase | units only |
| #33 | Least-privilege app permissions and a security review | none | `e2e/tests/secrets/github_no_credential_in_worker` | e2e only |
| #34 | Install guide from zero to a first team | `crates/team/tests/readme_guide.rs::install_doc_says_the_cli_is_fantastics` | n/a (docs or design) | covered |
| #38 | Surfaces are fantastic's words: `--headless` (no screen, a local port), `canvas --port N` (web UI), the terminal UI by default | n/a (base's parser, base tests it) | n/a (base behaviour, base tests it) | n/a |
| #40 | Worker packs: pack and install a worker with its sub-agents; removed by #292 (export and import are fantastic's `user.export_subtree` / `user.import_subtree`) | none | n/a (removed by #292) | n/a |
| #41 | Pack evals and a signed measured record, local until published; removed by #292 (export and import are fantastic's `user.export_subtree` / `user.import_subtree`) | none | n/a (removed by #292) | n/a |
| #42 | Pack registry and listing: browse, install, update, report, take down; removed by #292 (export and import are fantastic's `user.export_subtree` / `user.import_subtree`) | none | n/a (removed by #292) | n/a |
| #46 | CI by hand: units by default, e2e with -f e2e=true | `crates/team/tests/ci_workflow.rs::ci_workflow_parses_and_is_dispatch_only`, `crates/team/tests/ci_workflow.rs::ci_workflow_has_e2e_boolean_input_default_false`, `crates/team/tests/ci_workflow.rs::ci_full_jobs_gated_on_e2e_and_units` | n/a (workflow file) | covered |
| #48 | Secured shift-to-shift links: design only | none | n/a (docs or design) | n/a |
| #49 | Remote runner through a tunnel | `crates/team_app/src/lib.rs::tunnel_args_forward_loopback_only`, `crates/team_app/src/lib.rs::launch_tunnel_spawns_ssh_with_forward_and_returns_local_url`, `crates/team_app/src/lib.rs::tunnel_target_parses_host_and_port_and_refuses_urls` | `e2e/tests/app/attach_remote` | covered |
| #51 | README for AIs: the doc, printed as a section of fantastic's `readme` | `crates/team/tests/cli_is_fantastics.rs::the_team_s_text_is_a_section_of_fantastic_s_readme`, `crates/team/src/main.rs::ai_readme_names_fantastic_s_command_line_and_the_removed_flags_only_as_removed` | n/a (docs or design) | covered |
| #52 | SaaS hosted shifts: design only | none | n/a (docs or design) | n/a |
| #53 | Later: a list of your teams (not built) | none | n/a (docs or design) | n/a |
| #54 | Paid authenticated relay (not built) | none | n/a (docs or design) | n/a |
| #56 | Rename the code from shift to AISixteen Fantastic Team | none | deferred to the e2e phase | missing |
| #57 | Cadencer: each lead gets a digest of the level below it, every minute | `crates/team/src/cadencer.rs::a_beat_sends_each_digest_to_its_driver`, `crates/team/src/cadencer.rs::quiet_mode_sends_nothing_without_change`, `crates/team/src/cadencer.rs::live_set_takes_effect_next_tick` | `e2e/tests/dispatcher/digest_reaches_driver`, `e2e/tests/dispatcher/quiet_heartbeat` | covered |
| #58 | Dispatcher is the main view: tree of teams, groups, workers; controls | `crates/team/src/dispatcher.rs::tree_groups_workers_under_groups_under_teams`, `crates/team/src/dispatcher.rs::open_group_returns_nested_dispatcher`, `crates/team/src/dispatcher.rs::control_via_kernel_stop_needs_confirm_then_reaches_the_node`, `crates/team/src/page.rs::index_is_the_team_page_with_the_shim_in_its_head` | `e2e/tests/dispatcher/stop_asks_then_stops`, `e2e/tests/dispatcher/hand_task_add_worker` | covered |
| #80 | README plays the video inline; the video was later deleted by #152 | none | n/a (superseded by #152) | n/a |
| #95 | Group driver wakes only on a digest or an escalation | none (the unwired driver library went in #292) | `e2e/tests/dispatcher/digest_reaches_driver` | e2e only |
| #96 | Design briefs: idempotent task that carves screens into design/ | `tests/design_briefs.sh::first_run_writes_everything`, `tests/design_briefs.sh::second_run_changes_nothing`, `tests/design_briefs.sh::check_passes_good_briefs` | n/a (shell script; the shell test runs it for real) | covered |
| #101 | Cargo.lock refreshed once; Go 1.21 or later documented (the Go note has no test) | `crates/team/tests/fantastic_pin.rs::lock_pins_one_fantastic_commit` | n/a (build config) | units only |
| #102 | A new task starts the worker with a clear context | none | deferred to the e2e phase | missing |
| #109 | Dispatcher: groups are a record field of the dispatcher | `crates/team/src/dispatcher.rs::control_via_kernel_add_worker_creates_the_agent_and_lists_it_in_the_tree` | `e2e/tests/dispatcher/digest_reaches_driver` | covered |
| #112 | Cadencer on a live kernel: timer starts, set accepted, tick yields a digest | `crates/team/src/cadencer.rs::dispatcher_ticks_on_its_own_timer`, `crates/team/src/cadencer.rs::set_refused_from_a_non_parent` | `e2e/tests/dispatcher/digest_reaches_driver` | covered |
| #114 | Driver directive: clear and hand out in two turns | none | deferred to the e2e phase | missing |
| #115 | A member working on an inquiry shows as busy | `crates/team/src/cadencer.rs::digests_follow_real_work`, `crates/team/src/cadencer.rs::open_inquiry_comes_from_the_status_of_the_leads` | deferred to the e2e phase | units only |
| #119 | SIGTERM stops the `--headless` binary | `crates/team/tests/shutdown_bounded.rs::sigterm_stops_a_headless_team` | n/a (shutdown_bounded.rs spawns the real binary) | covered |
| #120 | Cadencer settings and timer survive a restart | `crates/team/tests/cadencer_live.rs::configure_survives_restart`, `crates/team/tests/cadencer_live.rs::set_survives_restart`, `crates/team/tests/cadencer_live.rs::off_list_survives_restart` | deferred to the e2e phase | units only |
| #121 | github connector creates, closes and reopens issues as the app | `crates/github_connector/src/tests.rs::issue_create_posts_as_the_app_and_adds_to_board`, `crates/github_connector/src/tests.rs::issue_close_then_reopen_round_trips`, `crates/github_connector/src/tests.rs::closing_an_epic_is_refused_unless_maintainer`, `crates/github_connector/src/tests.rs::issue_create_and_state_refuse_repo_outside_allowlist` | deferred to the e2e phase | units only |
| #122 | App sidebar: workers and subagents, collapsed by default; an agent opens full screen | `crates/team/src/dispatcher.rs::sidebar_subagent_lists_start_collapsed`, `crates/team/src/dispatcher.rs::expanding_one_worker_keeps_others_collapsed`, `crates/team/src/dispatcher.rs::open_worker_or_subagent_returns_fullscreen_chat_targeting_only_it`, `crates/team/src/dispatcher.rs::html_sidebar_has_expander_and_remembers_state` | `e2e/tests/ui/sidebar_opens_agent_full_screen` | covered |
| #127 | Load view: code gives numbers, the LLM decides what to show | `crates/team/tests/load_view.rs::no_mode_field_or_enum_for_load`, `crates/team/src/dispatcher.rs::html_draws_team_load_footer` | `e2e/tests/dispatcher/digest_reaches_driver` | covered |
| #146 | README video back as an inline preview; later deleted by #152 | none | n/a (superseded by #152) | n/a |
| #152 | Delete the how-it-works video and GIF from the repo and README | none | deferred to the e2e phase | missing |
| #164 | CI cleanup steps optional, off by default | none | n/a (workflow file) | missing |
| #180 | App launcher is ready when the page path answers, not only / | `crates/team_app/src/lib.rs::wait_ready_accepts_a_server_that_404s_at_root_and_200s_on_the_page_path`, `crates/team_app/src/lib.rs::launch_start_is_ready_when_only_the_page_path_answers_200` | `e2e/tests/app/launcher_start_attach_stop` | covered |
| #181 | --port serves /team_page and /team_ws so the worker view and sidebar open | `crates/team/src/app_control.rs::a_fresh_directory_serves_team_page_on_the_first_start`, `crates/team/src/app_control.rs::the_page_socket_reaches_the_dispatcher` | `e2e/tests/ui/worker_view_opens` | covered |
| #185 | Roles: coder role set, team limits, one store: role bundles, the registry's limits, the spawner's model, spawn from the new team | `crates/team/src/roles.rs::cards_parse_and_have_the_five_coder_names_plus_project_lead`, `crates/team/src/new_team.rs::the_lead_spawns_a_worker_and_a_project_lead_through_the_registry`, `crates/team/tests/readme_guide.rs::every_record_field_the_readme_names_is_read_by_the_code` | `e2e/tests/roles/total_limit_refuses_fourth_worker`, `e2e/tests/roles/project_lead_plans_and_runs_issues`, `e2e/tests/roles/one_store_survives_restart` | covered |
| #224 | `webui` recipe word serves the team page from the binary; `webui` with `canvas` refused | `crates/team/src/webui.rs::the_word_takes_port_and_host_in_any_order`, `crates/team/src/webui.rs::canvas_and_run_do_not_go_with_it`, `crates/team/tests/webui.rs::webui_port_serves_the_team_page_and_its_socket`, `crates/team/tests/webui.rs::webui_and_canvas_together_are_refused_before_anything_is_written` | deferred to the e2e phase | units only |
| #303 | GitHub auth by configuration: personal token or own app by env refs, local only without one | `crates/github_connector/src/tests_auth.rs::personal_token_is_used_as_is_with_no_mint`, `crates/github_connector/src/tests_auth.rs::no_credential_means_local_only_for_every_verb`, `crates/github_connector/src/tests_auth.rs::the_wrong_mode_s_missing_field_is_refused_by_name`, `crates/team/src/agent_views.rs::github_view_shows_the_auth_mode_and_the_local_note` | `e2e/tests/task/github_auth_by_env_ref` | covered |

## Gaps still open

The issues the coverage check reopened (see #147), and the rows above that are `missing`. A line is ticked only when the test it asks for exists on main.

- [x] #28: units for `issue_edit` and `project_field_set` now exist (`crates/github_connector/src/tests.rs`); its e2e is still deferred to the e2e phase.
- [ ] #36: the worker view opens over the web UI in `e2e/tests/ui/worker_view_opens`, but #36 is open now with a different title (agent views), so it is not a row above; revisit when it closes.
- [x] #57: `e2e/tests/dispatcher/digest_reaches_driver` runs the cadencer in the built binary and sees a digest reach a driver.
- [x] #122: `e2e/tests/ui/sidebar_opens_agent_full_screen` runs the page's own script against the live socket (no real browser; a stub DOM).
- [ ] #7: the queue bundle is fantastic's; this repo has an e2e but no unit test of take, renew and done.
- [ ] #56: no test that the old "shift" name is gone from the code.
- [ ] #102: no test that a new task starts the worker with a clear context.
- [ ] #114: no test that the driver directive clears and hands out in two turns.
- [ ] #152: no test that the README and repo hold no how-it-works video or GIF.
- [ ] #164: no test that the CI `cleanup` input exists and defaults to false.

## Unit coverage numbers

`cargo llvm-cov` is not installed on the machine that wrote this file, so no number below is measured. Nothing here is estimated.

| crate or package | line coverage | branch coverage |
| --- | --- | --- |
| team | not measured yet (cargo llvm-cov not available) | not measured yet |
| team_app | not measured yet (cargo llvm-cov not available) | not measured yet |
| git_connector | not measured yet (cargo llvm-cov not available) | not measured yet |
| github_connector | not measured yet (cargo llvm-cov not available) | not measured yet |
| shell scripts (`tests/*.sh`) | not measured yet | not measured yet |

The CI gate that fails when a crate's coverage drops below the recorded number is not built yet.

## Unreached public functions and error paths

Not yet inventoried: it needs the llvm-cov run above. When it exists, each public function, verb handler and error path no unit test reaches gets a test or a line saying why not.
