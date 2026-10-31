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
| #3 | No secret ever leaves an agent (secrets guard) | `crates/secrets_guard/tests/guard.rs::a_secret_in_a_reply_is_caught`, `crates/secrets_guard/tests/guard.rs::a_base64_secret_in_a_reply_is_caught`, `crates/secrets_guard/tests/guard.rs::a_clean_agent_passes`, `crates/secrets_guard/tests/check_secrets_script.rs::every_planted_secret_is_caught` | `e2e/tests/secrets/worker_never_sees_tokens` | covered |
| #4 | Headless fantastic host plus this repo's bundles | `crates/team/src/lib.rs::a_headless_host_boots_and_answers_reflect`, `crates/team/src/lib.rs::no_surface_flag_is_fully_headless` | `e2e/tests/surfaces/headless_binds_no_port` | covered |
| #5 | git connector: worktrees, branches, commits, push; credentials by ref | `crates/git_connector/src/tests.rs::worktree_add_makes_a_branch_from_base`, `crates/git_connector/src/tests.rs::push_never_forces`, `crates/git_connector/src/tests_secrets.rs::the_https_token_never_leaves_the_agent` | `e2e/tests/task/one_issue_to_draft_pr` | covered |
| #6 | github connector bound in team.yaml; token minted per call | `crates/github_connector/src/tests.rs::a_token_is_minted_per_call_for_the_repo`, `crates/github_connector/src/tests.rs::pr_create_is_a_draft_on_the_default_branch`, `crates/github_connector/src/tests.rs::repos_off_the_allowlist_are_refused_before_any_request` | `e2e/tests/task/github_pr_through_connector` | covered |
| #7 | Queue agent: tasks as data, leased take/renew/done (the queue bundle is fantastic's; only lease behaviour of peers has units here) | `crates/team/src/peer_transport.rs::a_lease_expires_after_its_ttl`, `crates/team/src/peer_transport.rs::renew_keeps_a_lease` | `e2e/tests/task/one_issue_to_draft_pr` | e2e only |
| #8 | Worker exec: no forge credentials, forge CLIs refused | `crates/team/tests/worker_exec.rs::a_push_from_a_worker_sends_no_credentials`, `crates/team/tests/worker_exec.rs::gh_version_is_not_refused_in_the_general_build`, `tests/build_github.sh::the_real_patch_applies` | `e2e/tests/exec/github_refuses_forge_calls`, `e2e/tests/exec/general_allows_gh` | covered |
| #9 | team.yaml: templates, connector bindings, queue location; unknown keys refused | `crates/team/src/config.rs::the_example_parses`, `crates/team/src/config.rs::unknown_keys_are_refused`, `crates/team/src/config.rs::connector_and_credential_records_are_exact` | `e2e/tests/task/one_issue_to_draft_pr` | covered |
| #10 | PR footer: lane, model, time, tokens, cost | `crates/github_connector/src/footer.rs::footer_from_fixed_usage_record`, `crates/github_connector/src/footer.rs::pr_create_appends_footer_when_usage_given`, `crates/github_connector/src/footer.rs::footer_contains_no_secret_like_values` | deferred to the e2e phase | units only |
| #11 | Task templates per kind and the generator | `tests/new_task.sh::every_kind_is_listed`, `tests/new_task.sh::fields_and_defaults_are_filled`, `tests/new_task.sh::no_template_holds_a_secret_like_string` | n/a (shell script; the shell test runs it for real) | covered |
| #12 | How-it-works animation (manim to mp4); the file was later deleted by #152 | none | n/a (superseded by #152) | n/a |
| #14 | Directives as files: worker and lead directives load | `crates/team/src/roles.rs::the_repo_roles_folder_loads_clean`, `crates/team/src/roles.rs::cards_parse_and_have_the_five_coder_names_plus_project_lead` | `e2e/tests/roles/project_lead_plans_and_runs_issues` | covered |
| #15 | Weak binding to fantastic: one pinned commit in Cargo.lock | `crates/team/tests/fantastic_pin.rs::lock_pins_one_fantastic_commit`, `crates/team/tests/fantastic_pin.rs::notes_name_pinned_commit` | n/a (build config) | covered |
| #16 | First agent's instructions via flags and file; counts validated | `crates/team/src/instructions.rs::given_instructions_then_rendered_text`, `crates/team/src/instructions.rs::instructions_file_read_and_used`, `crates/team/src/instructions.rs::refuses_zero_counts`, `crates/team/src/tree.rs::instructions_sent_only_on_first_create_or_when_asked` | deferred to the e2e phase | units only |
| #21 | Remote start over SSH, keys piped on stdin, never on disk | `tests/remote_scripts.sh::start_pipes_keys_on_stdin_not_argv`, `tests/remote_scripts.sh::start_refuses_missing_key`, `tests/remote_scripts.sh::no_file_holds_the_key` | n/a (shell script; the shell test runs it for real) | covered |
| #22 | Team tree built from config; tools created under the first agent | `crates/team/src/tree.rs::sample_config_plan_ids_parents_grants`, `crates/team/src/tree.rs::lead_is_created_first`, `crates/team/src/tree.rs::apply_twice_is_idempotent`, `crates/team/src/tree.rs::template_with_undefined_tool_refused_at_start` | `e2e/tests/task/one_issue_to_draft_pr` | covered |
| #25 | Puller replaces the dispatcher: labelled issues become task files | `crates/team/src/puller.rs::accepts_issue_labelled_by_maintainer_with_maintainer_spec`, `crates/team/src/puller.rs::rejects_label_added_by_non_maintainer`, `crates/team/src/puller.rs::pull_does_not_overwrite_existing_state`, `crates/github_connector/src/tests.rs::label_events_returns_only_label_events_with_actors` | `e2e/tests/task/one_issue_to_draft_pr` | covered |
| #28 | Issue-keeper role: edit one issue line, set a board field, never touch the SPEC | `crates/github_connector/src/tests.rs::issue_edit_replaces_exactly_the_one_matching_line`, `crates/github_connector/src/tests.rs::issue_edit_refuses_a_line_inside_the_spec_section`, `crates/github_connector/src/tests.rs::project_field_set_sends_one_graphql_mutation_with_item_field_value`, `crates/github_connector/src/verbs.rs::spec_section_is_detected_until_next_heading` | deferred to the e2e phase | units only |
| #29 | Team board: pulls open issues, tracks claims, audits, asks, never assigns | `crates/team/src/board.rs::pull_caches_all_open_issues_from_mock_github`, `crates/team/src/board.rs::audit_never_assigns`, `crates/team/src/board.rs::audit_flags_double_claim`, `crates/team/src/board.rs::ask_is_recorded_with_answer` | `e2e/tests/dispatcher/digest_reaches_driver` | covered |
| #30 | App registration for third parties: manifest flow | `crates/team/src/setup_github.rs::manifest_has_exactly_the_least_privilege_permissions`, `crates/team/src/setup_github.rs::callback_with_wrong_state_is_refused`, `crates/team/src/setup_github.rs::key_goes_to_credential_store_only`, `crates/team/src/setup_github.rs::no_key_in_any_output` | `e2e/tests/app/connector_setup` | covered |
| #31 | Per-repo configuration in .github/team.yaml | `crates/team/src/repo_config.rs::parses_full_config`, `crates/team/src/repo_config.rs::rejects_unknown_key`, `crates/team/src/repo_config.rs::rejects_parent_dir_in_paths`, `crates/team/src/puller.rs::repo_without_config_is_not_served` | deferred to the e2e phase | units only |
| #32 | Webhook events trigger a board pull; signatures checked | `crates/team/src/webhook.rs::valid_signature_is_accepted`, `crates/team/src/webhook.rs::bad_signature_is_refused`, `crates/team/src/webhook.rs::issue_event_triggers_board_pull`, `crates/team/src/webhook.rs::secret_never_in_reply` | deferred to the e2e phase | units only |
| #33 | Least-privilege app permissions and a security review | `crates/team/src/setup_github.rs::manifest_has_exactly_the_least_privilege_permissions` | `e2e/tests/secrets/github_no_credential_in_worker` | covered |
| #34 | Install guide from zero to a first team | `crates/team/tests/readme_guide.rs::install_guide_names_the_setup_command`, `crates/team/tests/readme_guide.rs::install_help_block_matches_usage` | n/a (docs or design) | covered |
| #38 | Surfaces are opt-in: headless by default, --port web UI, --tui terminal UI | `crates/team/src/lib.rs::no_surface_flag_is_fully_headless`, `crates/team/src/lib.rs::port_serves_the_web_ui_on_that_port`, `crates/team/src/lib.rs::tui_opens_the_terminal_ui`, `crates/team/src/lib.rs::bad_surface_flags_are_refused` | `e2e/tests/surfaces/headless_binds_no_port`, `e2e/tests/surfaces/tui_without_terminal_refuses` | covered |
| #40 | Worker packs: pack and install a worker with its sub-agents | `crates/team/src/pack_manifest.rs::a_full_manifest_round_trips`, `crates/team/src/pack_manifest.rs::a_manifest_with_a_key_shaped_value_is_refused`, `crates/team/src/pack_bundle.rs::pack_strips_bindings_and_credentials`, `crates/team/src/tree.rs::applied_pack_install_builds_the_blueprint_worker` | `e2e/tests/packs/pack_install_roundtrip`, `e2e/tests/packs/install_blueprint_approve`, `e2e/tests/packs/enable_needs_bindings` | covered |
| #41 | Pack evals and a signed measured record, local until published | `crates/team/src/pack_eval.rs::all_passing_gives_rate_one`, `crates/team/src/pack_eval.rs::a_timeout_is_a_fail`, `crates/team/src/pack_eval.rs::summary_text_names_rate_time_and_cost`, `crates/team/src/pack_record.rs::signed_record_verifies`, `crates/team/src/pack_record.rs::nothing_is_publishable_without_opt_in` | `e2e/tests/packs/failing_evals_block_enable`, `e2e/tests/packs/eval_rerun_after_model_change`, `e2e/tests/packs/install_reports_evals_and_verdict`, `e2e/tests/packs/measured_record_stays_local_until_published`, `e2e/tests/packs/shifts_produce_verifiable_record`, `e2e/tests/packs/ui_shows_pass_rate_time_cost` | covered |
| #42 | Pack registry and listing: browse, install, update, report, take down | `crates/team/src/pack_hosted.rs::fetch_index_accepts_a_valid_signed_index`, `crates/team/src/pack_hosted.rs::fetch_index_skips_a_tampered_entry`, `crates/team/src/pack_page.rs::page_escapes_html_in_every_field`, `crates/team/src/pack_browser.rs::update_diff_hidden_until_opt_in`, `crates/team/src/app_control.rs::page_leg_may_call_every_verb_dispatcher_html_sends` | `e2e/tests/packs/pack_publish_install_update`, `e2e/tests/packs/listing_page`, `e2e/tests/packs/install_from_hosted_registry`, `e2e/tests/packs/public_publish_review`, `e2e/tests/packs/report_and_takedown`, `e2e/tests/packs/web_browser` | covered |
| #46 | CI by hand: units by default, e2e with -f e2e=true | `crates/team/tests/ci_workflow.rs::ci_workflow_parses_and_is_dispatch_only`, `crates/team/tests/ci_workflow.rs::ci_workflow_has_e2e_boolean_input_default_false`, `crates/team/tests/ci_workflow.rs::ci_full_jobs_gated_on_e2e_and_units` | n/a (workflow file) | covered |
| #48 | Secured shift-to-shift links: design only | none | n/a (docs or design) | n/a |
| #49 | Remote runner through a tunnel | `crates/team_app/src/lib.rs::tunnel_args_forward_loopback_only`, `crates/team_app/src/lib.rs::launch_tunnel_spawns_ssh_with_forward_and_returns_local_url`, `crates/team_app/src/lib.rs::tunnel_target_parses_host_and_port_and_refuses_urls`, `tests/remote_scripts.sh::attach_dry_run_prints_the_tunnel` | `e2e/tests/app/attach_remote` | covered |
| #51 | README for AIs: the ai-readme subcommand and its doc | `crates/team/src/main.rs::ai_readme_subcommand_prints_the_doc_file`, `crates/team/src/main.rs::ai_readme_names_every_subcommand_and_flag`, `crates/team/tests/readme_guide.rs::ai_readme_section_8_names_the_pack_cli` | n/a (docs or design) | covered |
| #52 | SaaS hosted shifts: design only | none | n/a (docs or design) | n/a |
| #53 | Later: a list of your teams (not built) | none | n/a (docs or design) | n/a |
| #54 | Paid authenticated relay (not built) | none | n/a (docs or design) | n/a |
| #56 | Rename the code from shift to AISixteen Fantastic Team | none | deferred to the e2e phase | missing |
| #57 | Cadencer: each lead gets a digest of the level below it, every minute | `crates/team/src/cadencer.rs::a_beat_sends_each_digest_to_its_driver`, `crates/team/src/cadencer.rs::quiet_mode_sends_nothing_without_change`, `crates/team/src/cadencer.rs::live_set_takes_effect_next_tick`, `crates/team/tests/cadencer_live.rs::live_board_and_cadencer_deliver_through_a_real_kernel` | `e2e/tests/dispatcher/digest_reaches_driver`, `e2e/tests/dispatcher/quiet_heartbeat` | covered |
| #58 | Dispatcher is the main view: tree of teams, groups, workers; controls | `crates/team/src/dispatcher.rs::tree_groups_workers_under_groups_under_teams`, `crates/team/src/dispatcher.rs::open_group_returns_nested_dispatcher`, `crates/team/src/dispatcher.rs::control_via_kernel_stop_needs_confirm_then_reaches_the_node`, `crates/team/src/page.rs::index_is_the_dispatcher_with_the_shim_in_its_head` | `e2e/tests/dispatcher/stop_asks_then_stops`, `e2e/tests/dispatcher/hand_task_add_worker`, `e2e/tests/dispatcher/chat_features`, `e2e/tests/dispatcher/chat_history_send_stream` | covered |
| #80 | README plays the video inline; the video was later deleted by #152 | none | n/a (superseded by #152) | n/a |
| #95 | Group driver wakes only on a digest or an escalation | `crates/team/src/driver/mod.rs::driver_wakes_only_on_digest_or_escalation`, `crates/team/src/driver/mod.rs::driver_does_not_poll`, `crates/team/src/driver/mod.rs::driver_context_stays_within_cap`, `crates/team/src/driver/escalate.rs::chain_always_ends_at_maintainer`, `crates/team/src/driver_config.rs::over_budget_driver_stops_waking` | `e2e/tests/dispatcher/digest_reaches_driver` | covered |
| #96 | Design briefs: idempotent task that carves screens into design/ | `tests/design_briefs.sh::first_run_writes_everything`, `tests/design_briefs.sh::second_run_changes_nothing`, `tests/design_briefs.sh::check_passes_good_briefs` | n/a (shell script; the shell test runs it for real) | covered |
| #101 | Cargo.lock refreshed once; Go 1.21 or later documented (the Go note has no test) | `crates/team/tests/fantastic_pin.rs::lock_pins_one_fantastic_commit` | n/a (build config) | units only |
| #102 | A new task starts the worker with a clear context | none | deferred to the e2e phase | missing |
| #109 | Board: groups and drivers from config so the cadencer can deliver | `crates/team/src/board.rs::rows_list_configured_groups_and_members`, `crates/team/src/board.rs::team_groups_survive_a_real_kernel_create`, `crates/team/src/tree.rs::plan_fills_dispatcher_groups_from_config` | `e2e/tests/dispatcher/digest_reaches_driver` | covered |
| #112 | Cadencer on a live kernel: timer starts, set accepted, tick yields a digest | `crates/team/tests/cadencer_live.rs::live_board_and_cadencer_deliver_through_a_real_kernel`, `crates/team/src/cadencer.rs::dispatcher_ticks_on_its_own_timer`, `crates/team/src/cadencer.rs::set_refused_from_a_non_parent` | `e2e/tests/dispatcher/digest_reaches_driver` | covered |
| #114 | Driver directive: clear and hand out in two turns | none | deferred to the e2e phase | missing |
| #115 | A member working on an inquiry shows as busy | `crates/team/src/board.rs::a_member_with_an_open_inquiry_is_busy_with_that_task`, `crates/team/src/board.rs::a_member_generating_is_busy`, `crates/team/src/cadencer.rs::digests_follow_real_work` | deferred to the e2e phase | units only |
| #119 | SIGTERM and SIGINT stop the binary within 5 seconds | `crates/team/tests/shutdown_bounded.rs::sigterm_exits_within_5s_idle`, `crates/team/tests/shutdown_bounded.rs::sigterm_exits_within_5s_with_inflight_model_call`, `crates/team/tests/shutdown_bounded.rs::sigint_exits_within_5s_with_inflight_model_call`, `crates/team/src/watchdog.rs::exit_code_maps_signals` | n/a (shutdown_bounded.rs spawns the real binary) | covered |
| #120 | Cadencer settings, board and timer survive a restart | `crates/team/tests/cadencer_live.rs::configure_survives_restart`, `crates/team/tests/cadencer_live.rs::set_survives_restart`, `crates/team/tests/cadencer_live.rs::off_list_survives_restart` | deferred to the e2e phase | units only |
| #121 | github connector creates, closes and reopens issues as the app | `crates/github_connector/src/tests.rs::issue_create_posts_as_the_app_and_adds_to_board`, `crates/github_connector/src/tests.rs::issue_close_then_reopen_round_trips`, `crates/github_connector/src/tests.rs::closing_an_epic_is_refused_unless_maintainer`, `crates/github_connector/src/tests.rs::issue_create_and_state_refuse_repo_outside_allowlist` | deferred to the e2e phase | units only |
| #122 | App sidebar: workers and subagents, collapsed by default; an agent opens full screen | `crates/team/src/dispatcher.rs::sidebar_subagent_lists_start_collapsed`, `crates/team/src/dispatcher.rs::expanding_one_worker_keeps_others_collapsed`, `crates/team/src/dispatcher.rs::open_worker_or_subagent_returns_fullscreen_chat_targeting_only_it`, `crates/team/src/dispatcher.rs::html_sidebar_has_expander_and_remembers_state` | `e2e/tests/ui/sidebar_opens_agent_full_screen` | covered |
| #127 | Load view: code gives numbers, the LLM decides what to show | `crates/team/tests/load_view.rs::no_mode_field_or_enum_for_load`, `crates/team/tests/load_view.rs::load_numbers_has_no_total_or_of_key`, `crates/team/src/board.rs::load_counts_from_a_fixed_board`, `crates/team/src/dispatcher.rs::html_draws_team_load_footer` | `e2e/tests/dispatcher/digest_reaches_driver` | covered |
| #136 | Two builds: general (stock exec) and GitHub (exec patch) | `tests/build_github.sh::the_real_patch_applies`, `tests/build_github.sh::general_source_has_no_forge_refusal`, `tests/build_github.sh::github_build_prints_patch_hash`, `crates/team/tests/build_scripts.rs::build_github_fails_on_a_bad_patch`, `crates/team/tests/ci_workflow.rs::ci_builds_and_tests_both_builds` | `e2e/tests/exec/general_allows_gh`, `e2e/tests/exec/github_refuses_forge_calls` | covered |
| #146 | README video back as an inline preview; later deleted by #152 | none | n/a (superseded by #152) | n/a |
| #152 | Delete the how-it-works video and GIF from the repo and README | none | deferred to the e2e phase | missing |
| #164 | CI cleanup steps optional, off by default | none | n/a (workflow file) | missing |
| #180 | App launcher is ready when the page path answers, not only / | `crates/team_app/src/lib.rs::wait_ready_accepts_a_server_that_404s_at_root_and_200s_on_the_page_path`, `crates/team_app/src/lib.rs::launch_start_is_ready_when_only_the_page_path_answers_200`, `crates/team_app/src/lib.rs::ready_path_matches_the_teams_page_path` | `e2e/tests/app/launcher_start_attach_stop` | covered |
| #181 | --port serves /team_page and /team_ws so the worker view and sidebar open | `crates/team/src/app_control.rs::start_with_a_port_serves_the_web_ui`, `crates/team/src/app_control.rs::a_fresh_directory_serves_team_page_on_the_first_start`, `crates/team/src/app_control.rs::the_page_socket_reaches_the_dispatcher` | `e2e/tests/app/handle_serves_port`, `e2e/tests/ui/worker_view_opens` | covered |
| #185 | Roles: coder role set, team limits, one store | `crates/team/src/roles.rs::cards_parse_and_have_the_five_coder_names_plus_project_lead`, `crates/team/src/config.rs::roles_limits_parse_and_unknown_keys_refused`, `crates/team/src/tree.rs::plan_has_exactly_one_store_agent`, `crates/team/src/tree.rs::boot_limits_are_applied` | `e2e/tests/roles/total_limit_refuses_fourth_worker`, `e2e/tests/roles/project_lead_plans_and_runs_issues`, `e2e/tests/roles/one_store_survives_restart` | covered |

## Gaps still open

The issues the coverage check reopened (see #147), and the rows above that are `missing`. A line is ticked only when the test it asks for exists on main.

- [x] #28: units for `issue_edit` and `project_field_set` now exist (`crates/github_connector/src/tests.rs`); its e2e is still deferred to the e2e phase.
- [ ] #36: the worker view opens over the web UI in `e2e/tests/ui/worker_view_opens`, but #36 is open now with a different title (agent views), so it is not a row above; revisit when it closes.
- [x] #38: `e2e/tests/surfaces/headless_binds_no_port` and `e2e/tests/surfaces/tui_without_terminal_refuses` assert no port bound and a refusal without a terminal.
- [x] #57: `e2e/tests/dispatcher/digest_reaches_driver` runs the cadencer in the built binary and sees a digest reach a driver.
- [x] #122: `e2e/tests/ui/sidebar_opens_agent_full_screen` runs the page's own script against the live socket (no real browser; a stub DOM).
- [x] #137: `tests/repro_scripts.sh::canvas_zip_is_byte_identical_across_two_packs` exists.
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
| secrets_guard | not measured yet (cargo llvm-cov not available) | not measured yet |
| shell scripts (`tests/*.sh`) | not measured yet | not measured yet |

The CI gate that fails when a crate's coverage drops below the recorded number is not built yet.

## Unreached public functions and error paths

Not yet inventoried: it needs the llvm-cov run above. When it exists, each public function, verb handler and error path no unit test reaches gets a test or a line saying why not.
