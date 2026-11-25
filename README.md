# Parachain Service

The parachain service lets you run Polkadot parachains on JAM. It:

- Replaces *candidate inclusion* with an implementation of JAM `accumulate`.
- Replaces off-chain validation of *candidate backing* with an implementation of JAM `refine`.
- Offloads *approval checking*, *availability* and *backing* to JAM.
- Exposes a new Cumulus interface for collators to author parachain blocks as Work Packages.

There is no longer a relay chain runtime; the former relay chain logic is implemented without FRAME
directly as an `accumulate` hook.

> The prototype implementation lives on the `parachain-service-poc` branch.

## Setup

Fetch the vendored reference and companion submodules once:

```sh
git submodule update --init
```

## Project Management

### Poc2Production issues

Issues labeled with this label are about transforming the mainly vibe-coded PoC into production ready code.
Fundamentally this means the code should be upstreamed to the respective project main/master branches. As the code
was mainly written by AI, it requires real human to understand, review and improve the code before they open the
upstream pull requests. There is no value in blindly upstreaming the code and putting the review burden on the reviewers,
 so please follow the rules :)

Otherwise the issues are free to pick by anyone, but please reach out in the issue or by DM before.
