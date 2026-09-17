# Reviewing a pull request in this fork

This repository is MegaETH's fork of the `op-revm` crate, taken from `rust/op-revm` of the OP monorepo.
`MEGAETH-FORK.md` holds the rules, the upstream touch-point table and the update procedures; this file is the reviewer's checklist against them.

## What every PR must satisfy

1. **Superset.** The public API of `op-revm` stays a superset of the upstream snapshot the fork is on.
   A PR that must break it says so and lands together with the consumer change (`mega-evm`, `mega-reth`).
2. **Only what the revm fork requires.** The fork changes upstream code where the MegaETH fork of revm changed a `Handler` method that `OpHandler` overrides, and nowhere else.
   MegaETH semantics belong in `mega-evm`; an OP behaviour change belongs upstream.
3. **Touch points.** Every upstream file the PR modifies has a row in the "Upstream touch points" table of `MEGAETH-FORK.md` with a rule the next snapshot can follow without the author.
   `scripts/mega/check-touch-points.sh` lists the rows that are missing; the reviewer checks the rules are followable.
4. **`no_std`.** No unguarded `std::`, no new dependency that enables `std` by default.
   CI checks both riscv targets; the reviewer checks the intent.
5. **One revm fork commit.** The `rev` in `.cargo/config.toml` is the only place that names the revm fork commit; a port PR bumps it in the same commit as the code it requires.
6. **One topic per PR, conventional prefix on every fork commit.** Commit and PR titles start with `feat`, `fix`, `chore`, `docs`, `ci` or `test` and say what changed; `git log --oneline <snapshot>..main` is the fork's changelog and must stay readable; snapshot commits are the exception and start with `Mirror op-revm@`.

## By kind of change

| Kind | Look for |
|---|---|
| Snapshot | The snapshot commit's subject names the monorepo commit; `scripts/mega/base.txt` and `PROVENANCE.md` name the same commit; the PR body carries the output of `check-mirror.sh`; every touch-point row was re-applied on top; nothing MegaETH-specific rides in the snapshot commit |
| Port | The change is forced by a `Handler` shape in the pinned revm fork commit; the OP rules it touches (deposit gas, refunds, operator fee) keep their behaviour, with a test for each path |
| CI or docs | Workflows pin action SHAs; nothing requires a secret; the local equivalent of every job is documented |

## What not to ask for

- Style changes to upstream code. The fork keeps the OP formatting (`rustfmt.toml` is the monorepo's) and does not rename or reorganise upstream code.
- MegaETH behaviour tests. They belong in `mega-evm`, which pins this fork by tag.
- Upstream feature work. A new OP hardfork or precompile arrives with the next snapshot, not as a fork commit.
