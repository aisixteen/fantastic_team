# packs/install_reports_evals_and_verdict (free)

Issue #41: install a pack that carries evals through the CLI (`fantastic_team packs install --worker lead`), on the scripted model.

Two packs, built with `packs pack`. `good` has one passing eval: the reply has `evals` with pass rate 1.0, a time, the cost key (null: the scripted model reports no cost) and the one-line text (pass rate, time, cost), and `enable` allows it with the rate in the reason. `mixed` has a passing and a failing eval: the install still exits 0 and keeps the template, the pass rate is 0.5 and `enable` blocks. The scripted model served exactly three tasks.
