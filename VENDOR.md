# Vendored dependencies

Upstream repos live under `vendor/` as git submodules. Init with:

```bash
git submodule update --init
```

`vendor/` is in `.gitignore`; the submodules are tracked anyway (committed gitlinks, added with
`git submodule add -f`). Anything else dropped in `vendor/` stays ignored.

## Submodules

| Path | Upstream | Branch | Commit | Used for |
|------|----------|--------|--------|----------|
| `polkajam` | `paritytech/polkajam` | `oty-parachain-service-companion` | `3c9387bf` | build: `jam-types`, `jam-pvm-common`, `jam-node`, `jam-program-blob-common`, `jam-std-common` |
| `polkadot-sdk-companion` | `paritytech/polkadot-sdk` | `oty-parachain-companion` | `cb9f3ded` | build: `sc-executor`, `sp-core`, `sp-io`, `sp-runtime`, `sp-state-machine`, `sp-version` |
| `polkadot-sdk-cumulus` | `paritytech/polkadot-sdk` | `mku-cumulus-on-jam-doc` | `3b934d1c` | reference: Cumulus-on-JAM design doc |
| `polkadot-sdk-dafny` | `paritytech/polkadot-sdk` | `bkchr-parachain-service-doc` (PR [#11883](https://github.com/paritytech/polkadot-sdk/pull/11883)) | `afe236db` | reference: Dafny spec |
| `graypaper` | `gavofyork/graypaper` | `main` | `8ab3542` | reference: JAM Gray Paper |

`cumulus` and `dafny` are separate checkouts of the same repo on different branches.

## Build patches

`companion` and `polkajam` carry edits the build needs (they're not plain upstream checkouts).
Those edits live in commits on their branches above, both pushed to their remotes, so
`git submodule update --init` reproduces them.
