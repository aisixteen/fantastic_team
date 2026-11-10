# cli/single_lead

One lead solves the shared fixture (a git repo with one failing test; solved = the test
passes on HEAD and the fix is committed), started the two ways #219 allows:

- A, the lead laid as a record file under the fixed id `lead`, before the first start of `fantastic_team --headless`.
- B, with the recipe: fantastic's recipe `--headless ai anthropic --model M --endpoint URL` (the
  root AI `ai`), then `send` of `create_agent` / `update_agent`.

Checks, each variant: the test passes on HEAD and fails on the start commit, the work tree is
clean, and the record set after is the record set before the task (nothing was added).

Mutation: drop the commit turn from `script.json` and "solved" fails; make the lead
`create_agent` something in a turn and "the records are the ones it started with" fails.
