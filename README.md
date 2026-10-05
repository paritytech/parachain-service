# Parachain Service PoC

Run Polkadot parachains on JAM: `refine` validates candidates, `accumulate`
handles inclusion and service state, and Cumulus lets collators author Work Packages.
JAM provides backing, availability, and approval checking without a relay-chain runtime.

## Build and test

```sh
git submodule update --init
cargo test
```

Run `just --list` for build and maintenance recipes. For Quint equivalence testing:

```sh
just quint-fuzz            # 100 traces, eight workers
just quint-fuzz --infinite # run until failure or interruption
```

See [Quint replay](QUINT_REPLAY.md) for prerequisites, fixtures, and failure replay.

Deploy an infinite campaign to the SSH alias `fuzzer2` using every available CPU:

```sh
ansible-galaxy collection install -r ansible/requirements.yml
ansible-playbook -i ansible/inventory.ini ansible/quint-fuzz.yml
ssh fuzzer2 'sudo journalctl -u quint-fuzz -f'
```

The playbook requires local Ansible and rsync, initialized submodules, and an
Ubuntu/Debian server with Python 3, Node >=18, npm, and sudo access (`-K` if needed).
On x86_64 the pinned PolkaVM simulator requires AVX2; the playbook checks this
before deployment. VMs must expose it through their CPU configuration (for
example, host-passthrough), otherwise the blob build crashes with `SIGILL`.
It copies the local checkout, including uncommitted changes and Git metadata,
into `~/parachain-service-quint-fuzz`; build outputs and local environment files
are excluded. Use this dedicated checkout only for the deployed campaign:
redeploying stops the previous run and replaces its source. Dependencies are
installed under `~/.local/share/quint-fuzz` and the existing Rust default is preserved.
The service compiles on first startup, survives SSH disconnects, and starts at
boot. It stops on a failure without automatically restarting; reports stay in
`~/quint-fuzz-failures` across deployments. Inspect them before restarting with
`sudo systemctl restart quint-fuzz`, or stop with `sudo systemctl stop quint-fuzz`.
For local runs, `QUINT_FUZZ_WORKERS` overrides the recipe's default of eight workers.

## Code and references

- [Service](service/src/lib.rs): [Refine](service/src/refine.rs) and
  [Accumulate](service/src/accumulate/mod.rs), with [integration tests](service/bin/tests).
- [Authorizer core](authorizer), with [ed25519](authorizer-ed25519) and
  [sr25519](authorizer-sr25519) verifiers.
- [Cumulus interface](cumulus) and [mock parachain runtime](runtimes/frameless).
- [PolkaJAM executor](tools/executor/src/polkajam.rs) for PVM blob tests.
- [Pinned design and Quint model](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam).
- [Implementation decisions](DECISIONS.md), [divergences](DIVERGENCE.md), and
  [genesis setup](GENESIS.md).
