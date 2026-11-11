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

Folds (team #323): after the commit the lead makes a thought and prints a large reply (an exec `run` of ~70k characters, over
fantastic's autofold at 16000 tokens); message 1 ends with `done`, the probe reads the two rows from the lead's history and
hands their ids to the plug (`{{late:row_big}}`, `{{late:row_thought}}`), and message 2 recalls both. Checks, from the request
bodies the plug recorded (`helpers/folds.py`): the large reply is whole in the request right after its call and only its anchor
from the next on; the thought never reaches the model whole, only its anchor; a recall reply is whole in the next request and an
anchor one request later; every anchor is byte-identical in every later request; the store keeps both rows whole.
