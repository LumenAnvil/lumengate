<!-- Thanks for contributing to Lumengate. -->

## What this changes

<!-- A short description, and the issue it closes if any (Closes #123). -->

## Why

<!-- The problem this solves. -->

## Checklist

- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` is clean
- [ ] `cargo test` passes
- [ ] `cargo build --target wasm32v1-none --release` succeeds
- [ ] Behaviour changes have tests; payout-math changes cover the boundaries
      (zero, repeated calls, remainders) and call `assert_invariants`
- [ ] Docs / README updated if the interface changed
