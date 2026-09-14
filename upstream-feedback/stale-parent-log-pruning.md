# Rejected-candidate log pruning: resolved

Quint `06c2a49202` specifies that a rejected candidate changes no state, including
log pruning and tentative upgrade expiry. Updating the regressed submodule pin
restores that behavior in Rust. The old pin `4a22816d19` pruned on rejection;
the seed-1 and seed-2 failures against that revision are historical.

`service/bin/tests/fixtures/quint/log_pruning.qnt` now generates the stale-parent
and newer-anchor shape as `stale_parent_seed_1_works`. Both sides retain the log.
The Rust accumulate tests separately cover stale parents, invalid codes and
restricted messages carrying an expired upgrade.

```sh
python3 scripts/generate-quint-replays.py
cargo test --profile testnet -p parachain-service-bin --test quint_replay log_pruning::
```
