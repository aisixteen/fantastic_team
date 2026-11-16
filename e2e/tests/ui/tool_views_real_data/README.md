# ui/tool_views_real_data (free)

The cadencer, the GitHub connector and the bundles registry, each opened from the tree (#351). Function only.

- cadencer: the record's settings (on, 60 s, 300 s, quiet off, `m_ads` switched off) and its counts; a click on Quiet, on `m_ads ×` and a typed cadence each change the cadencer (read back as the operator);
- GitHub: the auth mode, the repo and api url of the record, the connector's own call log newest first (one call refused), a pull request looked up in the view;
- bundles: the kinds the registry lists with their purposes, the running count, a picked kind's directive.

LIVE (one model, many view-models): each view watches its own agent and sets no timer (its served source is read), and a change made outside the page by an operator call shows within 3 s without a reload — the cadencer's quiet switch and stall threshold (`set`), the connector's newest recent call (`issues` on a repo outside the allowlist), the registry's running count and the picked kind's running agents (`spawn`, then `delete_agent`). The polls these views had re-read every 5 to 10 s.
