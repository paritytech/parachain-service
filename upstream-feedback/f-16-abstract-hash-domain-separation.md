# F-16: the model's `Hash` is not a hash — `h(x) == h(y)` for `x != y`
`Hash` is `{ hashBytes: int }` and three constructors map into it untagged, so head data `1`
(`headHash`, `types.qnt:191`), validation code `{ vchBytes: 1 }` (`vchAsHash`, `types.qnt:59`)
and KV key `List(1)` (`listHash`, `types.qnt:195`) all hash to `{ hashBytes: 1 }`. Disproof:
drop [`f-16-hash-injectivity.qnt`](./f-16-hash-injectivity.qnt) into `quint/` and run
`quint test hash_injectivity.qnt`. §5.1 step 3's parent-head check compares hashes, so a code
hash equal to a head hash makes the model accept a candidate it should reject.

**Spec feedback**: domain-separate the constructors, as `merkleHash` (`head_commitment.qnt:25-28`)
already does for leaves vs nodes. Model-side only — real `hash_raw(blob)` and `hash(head_data)`
are unprefixed and consensus-critical.
