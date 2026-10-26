# packs/listing_page

Issue #42. `pack_listing page` returns one static html document built from the signed index and the installed packs.

Free tier. Checks: an untrusted signer's pack is not listed (`No packs listed.`); once trusted and installed, the page shows the name and version, every port and every grant as a list item, the `installed` badge, and `not stated` for the price and the evals count (a signed entry carries neither); a port string holding markup (`@connector:<b>x</b>`) is escaped (`&lt;b&gt;`), never live; after `takedown` the section carries the `revoked` badge; the reply is `{html}` only and the page holds no `<script`.

Real rendering in a browser is not exercised: the page is checked as text.
