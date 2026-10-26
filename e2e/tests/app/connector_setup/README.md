# app/connector_setup (free)

The app's GitHub connector setup (`fantastic_team_app --setup-github NAME`) shows a manifest page that asks GitHub for the least the app needs, refuses a callback it did not start, and leaves no key on disk.

The app runs the headless binary's `setup github --name NAME` in the team's directory and prints the page URL the child prints. The probe boots a team once, stops it (the lock is free), and drives the page like a browser.

Asserts: the app prints a loopback URL and has one child; the page is a form that posts a manifest to `https://github.com/settings/apps/new?state=...` with a 128-bit random state; the manifest has the name the user chose, exactly the six permissions of `setup_github.rs` (`contents`, `issues`, `pull_requests` write; `metadata`, `checks`, `actions` read), no webhook (`hook_attributes: {active: false}`) and no event, a `redirect_url` on this machine's loopback, `public: false`; the page names no host but github.com and 127.0.0.1; another path is not served; a callback with a wrong `state` is refused ("State did not match") and the flow ends there (the app exits non-zero, the child is gone, the page closed); `team.yaml` has no `github_app_id`, and no file under the run holds the app key text, a `PRIVATE KEY` marker or the model key; a second setup, cancelled with SIGINT, exits 0 and leaves no child.

Not asserted: "the setup ends with the connector configured and no key on disk". That tail needs the manifest-code exchange to reach a fake forge, and `setup_github::run_cli` hard-codes `https://api.github.com` (`api_base`). Pointing it at a fake forge takes a new flag or environment variable, which is not added without the maintainer's word. The pieces are unit-tested (`setup_github.rs`: the exchange against a mock base, the key stored and not printed, the app id recorded).

Mutation: drop one entry of `WANT` in the probe (for example `actions`); the permissions check fails.
