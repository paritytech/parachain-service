# F-16: abstract hashes discard their domain, so ITF traces are not replayable
Not a model defect — every `Hash`-consuming site is fed by one constructor (`accumulate.qnt:357`
compares `headHash` to `headHash`), so the model's own comparisons are sound. But the images
overlap: `headHash(1)`, `vchAsHash({ vchBytes: 1 })` and `listHash(List(1))` all yield
`{ hashBytes: 1 }` (`types.qnt:191/59/195`, disproof in [`f-16-hash-injectivity.qnt`](./f-16-hash-injectivity.qnt)).

Replay runs the other way — `{ hashBytes: 1 }` back to 32 real bytes — and the bytes depend on the
domain the trace no longer records. In `paraRegisterOperateCleanupTest` frame 3, `{ hashBytes: 1 }`
is both a `preimageRegistry` key (64 KiB validation code) and a `parentHeadHash` (8-byte head).

**Spec feedback**: domain-tag the constructors, as `merkleHash` (`head_commitment.qnt:25-28`) already
does for leaves vs nodes. A harness can recover the domain from the ITF field path, but that is a
lookup table outside the spec that rots as the model grows.
