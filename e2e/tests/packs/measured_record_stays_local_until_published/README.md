# packs/measured_record_stays_local_until_published (free)

Issue #41: a pack's measured record is local until its owner publishes it.

`pack eval` with a throwaway `signing_key` (a random seed in `secrets/`, never printed) replies with a signed `measured` record. Then: its signature is in no file of the run dir, there is no `packs/index.json`, the listing is empty, and the fake forge got no request. `pack_listing publish` of the pack with the record but without `opt_in` publishes the pack and not the record (`measured_published: false`, reason `opt_in is not set`); `show` has no `measured`. A second pack, measured, published with `opt_in: true`: `measured_published` is true and `show` returns the record; only now the signature is in a file (`packs/index.json`), which also proves the file walk can see it.
