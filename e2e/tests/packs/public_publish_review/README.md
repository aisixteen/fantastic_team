# packs/public_publish_review

Issue #42. A pack is reviewed before it is listed: the same checks as an install, and a scan finding of any kind stops it.

Free tier. Local: `publish` refuses a pack with a finding (a code file, written by hand because `pack` refuses it) with the file and rule and leaves the index byte-identical; the `pack` verb refuses a directive holding a secret-looking line, and the refusal does not echo it. Hosted: with a `registry_url`, `submit` of the same pack answers `{listed: false, status: "held", findings}` and the fake registry sees NOTHING; `submit` of a clean pack goes on and reaches the fake registry (a connection and a failed TLS handshake, since the binary trusts only public roots, so `submit failed`). The signing seed is in no reply and not in the team log.

Not covered here: a `listed: true` reply needs a registry whose certificate the binary trusts; that is a unit item (`review_holds_a_pack_with_scan_findings`, `review_lists_a_clean_pack` in `pack_listing.rs`) until the binary can trust a test certificate.
