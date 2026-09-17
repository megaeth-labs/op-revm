# Provenance

This repository is MegaETH's fork of the `op-revm` crate.
`op-revm` has no standalone upstream repository: it lives in `rust/op-revm` of the OP monorepo.
The fork starts from a snapshot of that directory and carries MegaETH's commits on top; `MEGAETH-FORK.md` holds the rules, the touch points and the update procedure.

| | |
|---|---|
| Upstream repository | <https://github.com/ethereum-optimism/optimism> |
| Upstream path | `rust/op-revm` |
| Upstream commit | `scripts/mega/base.txt` (`f67d87cd5c9992b62d215c043917b94e3f019629`, `develop` of 2026-07-09, the revision `mega-reth` locks) |
| Crate version | `20.0.0` |
| Snapshot commit | the newest commit in `main` whose subject starts with `Mirror op-revm@` |

In the snapshot commit every file is a verbatim copy of upstream, except `Cargo.toml`: the monorepo's `workspace = true` inheritances are flattened to the concrete values the workspace root declares at the same commit, so the crate builds standalone.
`scripts/mega/mirror.sh` produces such a snapshot from a monorepo checkout, and `scripts/mega/check-mirror.sh` re-checks the committed one against upstream.

The crates.io package `op-revm 20.0.0` is a different package under the same name and version: it was published from an older monorepo commit, depends on revm 38 and maps `INTEROP` to `PRAGUE`.
Only the monorepo copy is compatible with the MegaETH fork of revm.
