# Upgrade activation discards the old code's release log

When the old active code is provided, activating an upgrade unrequests it and
retains its referencer and balance charge until expunge. The pinned Quint model
discards the resulting `ForgetAgainAt` notification. Rust now matches that
behavior by discarding activation logs at the package boundary, as it does for
expiry cleanup.

[Issue #36](https://github.com/paritytech/parachain-service/issues/36) proposes
preserving the cleanup notification for both activation and expiry. Revisit the
Rust compatibility behavior once that decision is reflected in the pinned model.

The `upgrades/provided_old_code_activation_works.itf.json` fixture covers
activation with provided old code. Its strict replay checks that the upgrade
activates, the old code remains referenced and charged while awaiting expunge,
and no cleanup log is emitted. The existing `upgrades/activation_works.itf.json`
fixture covers unprovided old code, which is removed and refunded immediately.

Regenerate and replay the upgrade fixtures with:

```sh
python3 -c 'import runpy; runpy.run_path("scripts/generate-quint-replays.py")["generate"]("upgrades")'
cargo test -p parachain-service-bin --test quint_replay upgrades::
```
