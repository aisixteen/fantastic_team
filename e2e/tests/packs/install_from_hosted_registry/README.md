# packs/install_from_hosted_registry

Issue #42. `fetch` finds a pack in the SIGNED index by name and pinned hash, refuses a revoked one, and only then downloads from the entry's https url.

Free tier: no model call, no key. A `FakeRegistry` (`helpers/registry.py`) is an https server with a throwaway self-signed certificate. The team binary trusts only the public web roots, so it does not trust it; that is asserted.

What this test covers, all before or at the edge of the network:

- a hash the index does not pin, a revoked entry, and a right hash with a wrong version are each refused with no connection to the registry at all, and nothing is staged;
- a live pinned entry goes on to the download: the registry sees a connection and a failed TLS handshake, never an HTTP request, and `fetch` answers `could not reach the registry` (no body echoed), staging nothing;
- a `registry_url` over `http://` is refused with `must be https`.

What it does NOT cover: the success path (download, hash match, stage, then `install` of the staged folder). That is covered by the units in `crates/team/src/pack_listing.rs` (`fetch_stages_a_pack_by_hash`, `fetch_refuses_a_download_that_differs_from_the_pin`) and cannot run end to end until the binary can trust a test certificate (an extra trusted root or an insecure switch). That would be new API, so it is an open decision, not done here.
