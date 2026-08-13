# Contributing to Lumengate

Thanks for taking an interest. Lumengate is a small contract that moves money, so the bar is
correctness first and feature breadth a distant second.

## Getting set up

```bash
rustup target add wasm32v1-none    # Rust 1.91+
cargo test                         # full suite, runs natively
stellar contract build             # produces the wasm
```

You need `stellar-cli` for the build; see the
[Stellar CLI docs](https://developers.stellar.org/docs/tools/cli).

## What a change needs

**Tests.** Every behavioural change needs tests, and anything touching the payout math needs
tests covering the boundaries: zero, repeated calls, uneven shares that leave a remainder, and
the summed invariants. The `assert_invariants` helper in `src/test.rs` checks that per-recipient
payouts sum to `total_released`, that nothing is paid out beyond what came in, and that the
splitter is never overdrawn — call it after any operation your test performs.

**No new SDK guesswork.** SDK APIs change across majors. If you are unsure whether a function
exists in the pinned version, check [docs.rs/soroban-sdk](https://docs.rs/soroban-sdk) for that
exact version rather than relying on an older example.

**Keep the invariants stated in the README true.** If a change makes one of them false, the
README has to change in the same commit, with the reasoning.

**Storage lifetime.** New state belongs in instance storage unless there is a specific reason
otherwise, and that reason has to be argued in the pull request. Splitting state across storage
types reintroduces the desync bug the current design rules out — see "Storage and state
archival" in the README.

## Good first issues

These are intentionally out of scope for the current version:

- **Mutable recipients.** Adding, removing or reweighting recipients after deployment. The hard
  part is not the access control but what happens to entitlements already accrued under the old
  weights; a design proposal should come before an implementation.
- **Multiple tokens.** One splitter routing several tokens, which means per-token accounting.
- **Chunked `release_all`.** Paying a recipient set larger than the current cap of 20 across
  several transactions without breaking idempotency.
- **Weighted governance** over any of the above.

## Pull requests

- Keep them focused; one concern per pull request.
- `cargo test` must pass and `stellar contract build` must succeed.
- Run `cargo fmt` before submitting.
- Explain *why* in the description, not just what. For anything touching `release`, spell out
  the argument for why funds cannot be lost, duplicated, or misdirected.

## Reporting a vulnerability

Please do not open a public issue for a security problem in the payout logic. Report it
privately to the repository owner first, with a description of the impact and, if possible, a
failing test.
