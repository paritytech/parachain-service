# sr25519 authorizer — blob-size experiment

> **Superseded, and kept for the measurement.** Phase 6a took the answer below and shipped it:
> `authorizer-sr25519/` is the productized crate, over the scheme-blind core in `authorizer/`.
> Two things differ there and are deliberate. The signing context is `b"substrate"`, not the
> `jam:parachain-service:aura` this crate verifies with — a collator signs through
> `Keystore::sr25519_sign`, which hard-codes it. And the blob is a little larger, because the
> shipping crates split scheme-blind code out into a separate crate; the sizes in the table below
> are this experiment's, not the shipping blobs'.

**Not part of the product.** This is a disposable copy of the `authorizer` crate with the
ed25519 signature check swapped for sr25519 and *nothing else changed*, kept so that the next
person who asks "why is the AURA authorizer on ed25519?" gets a number they can rebuild rather
than a claim they have to trust.

## The question

JAM caps authorizer code at `C_maxauthcodesize = 64,000` bytes
(`vendor/polkajam/crates/jam-types/src/simple.rs`). The shipping ed25519 authorizer is 59,328
bytes — about 4,700 to spare. sr25519 had been ruled out on the grounds that schnorrkel in the
PVM guest would not fit in that margin. Nobody had measured it.

## The answer

| variant | blob | headroom under 64,000 | gas for one `is_authorized` |
| --- | --- | --- | --- |
| ed25519 (`authorizer`, shipping) | **59,328** | 4,672 | 1,234,094 |
| sr25519 via `schnorrkel` (this crate) | **59,349** | 4,651 | ~1.09 M (1.076–1.097 M observed) |
| sr25519 via `sp-core` (see below) | **61,748** | 2,252 | ~1.09 M |

sr25519 costs essentially nothing. 19 of the schnorrkel route's 21-byte difference is just this
crate's longer name in the blob metadata; the compiled program is 2 bytes bigger
(`.polkavm`: 59,705 → 59,707). It is also *cheaper* in gas — about 12% less than ed25519, and
either way ~2% of the 50 M budget an `is_authorized` call gets.

Why it comes out even: sr25519 drops SHA-512 (~5.5 kB of `sha2::sha512::compress512`) and picks
up merlin/STROBE (~1 kB of `keccak::f1600` plus transcript code) and curve25519-dalek's
`precomputed-tables` basepoint table (7,680 bytes of rodata, 64 affine points), while the field
and scalar arithmetic is shared with the ed25519 build. The two roughly cancel.

Gas for sr25519 varies by a couple of percent run to run: schnorrkel signing is randomised, and the verifier's
variable-time double-scalar multiplication depends on the scalar values. ed25519's is constant
here only because the test's signature is deterministic.

Numbers measured on rustc/cargo 1.100.0-nightly, `production-authorizer` profile,
`schnorrkel` 0.11.5, `curve25519-dalek` 4.1.3. Re-measure after a toolchain or schnorrkel bump.

## Reproducing

```sh
# blob size
taskset -c 0-16 cargo build --release --offline \
    --manifest-path authorizer-sr25519-experiment/bin/Cargo.toml
ls -l authorizer-sr25519-experiment/target/release/build/*/out/*.jam

# gas, both blobs, through the PolkaJAM interpreter
taskset -c 0-16 cargo test --release --offline \
    --manifest-path authorizer-sr25519-experiment/bin/Cargo.toml -- --nocapture
```

If a blob comes out 0 bytes, delete `authorizer-sr25519-experiment/target/release/build/*` and
rebuild — the blob build script caches aggressively.

## Things that got in the way (so you do not rediscover them)

- **This is its own cargo workspace.** The repo root lists the directory under
  `[workspace] exclude`, so a normal `cargo build`/`cargo test`/`cargo clippy` at the root never
  builds it and the root `Cargo.lock` never mentions it. The price is that no `workspace = true`
  inheritance is available here: every dependency, profile and lint setting is spelled out in
  `Cargo.toml` and has to be kept in step with the shipping `authorizer` crate by hand, or the
  comparison stops being like-for-like. `profile.production` and `profile.production-authorizer`
  are copied verbatim for the same reason.
- **`Cargo.lock` is seeded from the root one.** Resolving from scratch fails offline: `jam-node`
  depends on a git `quinn`, and cargo cannot look up its branch without the network. Copying the
  root `Cargo.lock` in first gives cargo the pinned revision. If this lock ever needs rebuilding,
  `cp ../Cargo.lock Cargo.lock` and build again.
- **schnorrkel needs `default-features = false`.** Its defaults are `std` + `getrandom`, and a
  bare RISC-V PVM guest has no `getrandom` backend. `features = ["alloc"]` is enough.
- **No host randomness is needed on the verify path** — this was the thing most likely to be a
  hard stop, and it is not one. `PublicKey::verify_simple` is deterministic; `getrandom_or_panic`
  compiles without a backend and is never reached. Confirmed two ways: the guest ELF contains no
  `getrandom`/`OsRng`/`rand_core` symbols at all, and the blob authorizes a real signature inside
  the PVM without trapping (`bin/tests/gas.rs`).
- **The 32 KiB guest stack already in `src/lib.rs` is enough.** It was raised for
  curve25519-dalek's ed25519 Straus tables; schnorrkel's ristretto path fits in the same budget
  unchanged.
- **`bin/tests/gas.rs` drives the interpreter itself** instead of reusing `tools/executor` and
  `parachain-service-bin::mock`. That harness does not compile against the `vendor/polkajam`
  commit this branch is pinned to (`executor` imports `jam_node::vm::RefineCallContext{,Owned}`,
  which the vendored node only defines under `#[cfg(test)]`). Pre-existing, unrelated to sr25519,
  but it means the usual harness is not available here.

## The `sp-core` route

`sp_core::sr25519::Pair::verify` is literally the same three lines as `src/aura.rs`'s
`check_signature` — `schnorrkel::PublicKey::verify_simple` with the context `b"substrate"` — so
it can only ever be a superset of the schnorrkel route. Measured anyway, at 61,748 bytes: the
extra 2.4 kB is sp-core's own baggage, and it eats half the remaining headroom for nothing. Not
recommended; recorded so the comparison is on the record.

It is not wired up as a switchable variant because building it needs a change to the *shipping*
`tools/pvm-builder`. sp-core has a `[target.'cfg(not(substrate_runtime))'.dependencies]` entry on
`futures` with std features on, so the guest build fails with `can't find crate for 'std'` unless
rustc is passed `--cfg substrate_runtime`; `pvm_builder::build_authorizer` only sets that cfg for
`BlobType::Runtime`. To redo the measurement, temporarily add to `build_pvm_blob` in
`tools/pvm-builder/src/lib.rs`, next to the existing `BlobType::Runtime` cfg push:

```rust
if let Ok(extra) = std::env::var("PVM_BUILDER_EXTRA_CFG") {
    rustflags.push_str("\x1f--cfg\x1f");
    rustflags.push_str(&extra);
}
```

then swap this crate's `schnorrkel` dependency for `sp-core = { path =
"../vendor/polkadot-sdk-companion/substrate/primitives/core", default-features = false }`, replace
the body of `check_signature` with `sp_core::sr25519::Pair::verify`, set `SIGNING_CONTEXT` to
`b"substrate"` so the test still signs what the guest verifies, and build with
`PVM_BUILDER_EXTRA_CFG=substrate_runtime`. Revert the `pvm-builder` change afterwards.

## Recommendation

sr25519 is feasible within the 64,000-byte ceiling with no special measures, via raw `schnorrkel`
with `default-features = false, features = ["alloc"]`. Blob size is not a reason to prefer
ed25519, and the design doc should not say it is. If sr25519 is rejected, reject it on some other
ground.

Note that this experiment only swapped the verification primitive. An actual migration also has
to move collator key generation, the keystore, `parasim-tool` and the collator-set fixtures, and
sr25519 signing is randomised where ed25519 signing is deterministic — none of which is measured
here.
