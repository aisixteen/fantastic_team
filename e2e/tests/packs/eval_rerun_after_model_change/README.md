# packs/eval_rerun_after_model_change (free)

Issue #41: `fantastic_team packs eval --pack P --worker lead` runs the pack's evals on the lead and reports a pass rate. After the model changes the same command reports a new one.

Two passing evals run on the scripted model: rate 1.0. Then the lead's `endpoint` is changed (`update_agent`) to a port nothing listens on, standing in for a model that no longer answers the same way. The same eval is run again: the rate is different (0%), the text shows it, and the scripted model is not asked again.
