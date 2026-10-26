# packs/report_and_takedown

Issue #42. A pack is reported to the registry, and a version is taken down.

Free tier. `report` needs a `registry_url` (`no registry_url` without one) and refuses an empty reason, a reason over 2000 characters and an unsafe name before any connection; a valid report goes to the registry, which here is a `FakeRegistry` the binary does not trust (public roots only), so it answers `report failed` and the fake sees one connection and no request. `takedown` with a registry that cannot be reached still revokes locally: `{revoked, pushed: false}`, the error `revoked locally, but the registry push failed`, `packs/index.json` holds `revoked: true` and the re-signed entry still verifies (`list` shows it, not skipped); the installed pack gets a warning, and its hash cannot be fetched. A missing or non-key `signing_key` file leaves the index unchanged.

Not covered here: a report or a takedown push that the registry ACCEPTS (`reported: true`, `pushed: true`) needs a registry whose certificate the binary trusts; the units `report_posts_name_version_and_reason` and `takedown_revokes_locally_and_pushes_to_the_hosted_index` in `pack_listing.rs` cover them.
