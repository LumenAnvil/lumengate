# Lumengate

A deployable, reusable **payment splitter** for Stellar Soroban.

Configure a set of recipients and their share weights once, at deployment. From then on the
contract routes every token it receives to those recipients in proportion to their shares.
Anyone can trigger a payout; funds can only ever reach the recipients written into the
configuration.

Revenue sharing, team payouts, royalty splits and DAO distributions all reduce to the same
problem — split incoming funds by fixed shares — and Soroban has no ready-to-deploy contract
for it. OpenZeppelin's Stellar libraries cover tokens, access control and governance, but not
this. Lumengate is a single small contract that does exactly this and nothing else.

> **Unaudited.** This contract has not been audited. It moves funds. Read
> [Security](#security) before deploying it to a network where value is at stake.

## How it works

The splitter has no deposit function. It simply holds a balance of one token, and anyone can
send that token to its address by ordinary transfer. Recipients then pull what they are owed.

```
     payer ──transfer──▶ ┌───────────────┐ ──release(alice)──▶ alice   (5 shares)
     payer ──transfer──▶ │   Lumengate   │ ──release(bob)────▶ bob     (3 shares)
                         │  holds token  │ ──release(carol)──▶ carol   (2 shares)
                         └───────────────┘
```

## Verified toolchain

Every version below was checked against crates.io and the Stellar developer docs at the time
of writing, and the contract is built and tested with them.

| Component | Version | Notes |
| --- | --- | --- |
| `soroban-sdk` | **27.0.5** | Latest release. Stellar supports the two most recent majors (27 and 26). |
| Rust | **1.91.0** minimum | The SDK's MSRV. Built and tested here with 1.97.1. |
| Build target | **`wasm32v1-none`** | Requires Rust 1.84+, and is the only wasm target the Soroban runtime accepts. |
| Build command | **`stellar contract build`** | From `stellar-cli` **27.1.0**. Do **not** use `cargo build`. |
| Edition | 2021 | |

`Cargo.lock` is committed so the wasm hash is reproducible.

## Build, test, deploy

### Prerequisites

```bash
rustup target add wasm32v1-none
# stellar-cli: see https://developers.stellar.org/docs/tools/cli
```

### Test

The full suite runs natively against the SDK's test environment — no wasm build needed.

```bash
cargo test
```

### Build

```bash
stellar contract build
# -> target/wasm32v1-none/release/lumengate.wasm
```

### Deploy

Configuration happens in the contract's `__constructor`, so deployment and initialization are
a single atomic operation. Constructor arguments go after the `--` separator:

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/lumengate.wasm \
  --source-account deployer \
  --network testnet \
  -- \
  --token CDLZ...  \
  --recipients '[{"address":"GALICE...","shares":5},{"address":"GBOB...","shares":3}]'
```

There is no separate `initialize` call, and therefore no window in which an unconfigured
splitter exists on-chain for someone else to claim.

[`scripts/deploy.sh`](scripts/deploy.sh) runs an end-to-end example against testnet:
it creates accounts, issues a test asset, deploys the splitter, funds it, and releases.

### Invoke

```bash
# What is alice owed right now?
stellar contract invoke --id $SPLITTER --source-account anyone --network testnet \
  -- releasable --who GALICE...

# Pay alice. Any account can call this; the funds go to alice regardless.
stellar contract invoke --id $SPLITTER --source-account anyone --network testnet \
  -- release --who GALICE...

# Pay everyone.
stellar contract invoke --id $SPLITTER --source-account anyone --network testnet \
  -- release_all
```

## Interface

| Function | Description |
| --- | --- |
| `__constructor(token, recipients)` | Runs once at deployment. `recipients` is a list of `{address, shares}`. |
| `release(who) -> i128` | Pays `who` everything they are owed; returns the amount moved. |
| `release_all() -> i128` | Pays every recipient; returns the total moved. |
| `releasable(who) -> i128` | What `release(who)` would pay right now. |
| `shares(who) -> u32` | `who`'s share weight, or `0` if not a recipient. |
| `released(who) -> i128` | Total ever paid to `who`. |
| `total_released() -> i128` | Total ever paid to all recipients. |
| `total_received() -> i128` | Everything the splitter has ever received. |
| `recipients() -> Vec<Recipient>` | The configured recipients, in construction order. |
| `total_shares() -> u32` | Sum of all share weights. |
| `token() -> Address` | The token being routed. |

### Errors

| # | Error | Raised when |
| --- | --- | --- |
| 1 | `NoRecipients` | The recipient list is empty. |
| 2 | `TooManyRecipients` | More than 20 recipients. |
| 3 | `DuplicateRecipient` | An address appears twice. |
| 4 | `ZeroShares` | A recipient has a weight of zero. |
| 5 | `RecipientIsSplitter` | A recipient is the splitter itself. |
| 6 | `NotARecipient` | `release`/`releasable` called for an unconfigured address. |
| 7 | `MathOverflow` | Payout arithmetic would leave the `i128` range. No state changes. |

## The payout math

Write `R` for everything the splitter has ever received, `sᵢ` for recipient *i*'s share
weight, and `S` for the sum of all weights.

```
R  = balance(splitter) + total_released
Eᵢ = floor(R × sᵢ / S)          cumulative entitlement
owedᵢ = Eᵢ − releasedᵢ          what the next release pays
```

`R` is derived rather than stored. That is what makes the contract work without a deposit
function: any transfer into the splitter raises its balance and therefore raises `R`, with no
call required from the payer.

Four properties follow, and each has a test:

**`R` never decreases.** A release lowers the balance by exactly the amount it adds to
`total_released`, leaving `R` untouched. Only an incoming transfer moves it. So `Eᵢ` is
non-decreasing and `owedᵢ` is never negative.

**No double payment.** `releasedᵢ` is only ever set to `Eᵢ`, a pure function of `R`. Calling
`release` twice with no deposit in between computes the same `Eᵢ` and pays `0`. Releases are
idempotent, so a failed or repeated transaction cannot pay anyone twice.

**Never overdrawn.** Since `Σ Eᵢ ≤ R`, the held balance `R − Σ releasedᵢ` always covers what
every recipient is owed. A payout can never fail for insufficient funds.

**Order does not matter.** Because paying one recipient leaves `R` unchanged, `release_all`
distributes exactly what the same recipients would get from individual calls in any order.

### The dust rule

Division is floor division, so a distribution can leave a remainder. **The remainder stays in
the contract.**

It is not lost. Because entitlements are computed on the cumulative `R`, held-back units are
folded into everyone's entitlement as soon as more funds arrive. Splitting 100 three ways pays
33 each and holds 1; after another 200 arrives, each recipient's entitlement is
`floor(300/3) = 100`, and the held unit goes out with the rest.

The undistributed remainder is `Σ frac(R × sᵢ / S)`, which is always **less than one base unit
per recipient** — at most 19 stroops with the maximum of 20 recipients. That residue only rests
permanently if no further deposit ever arrives.

### Overflow

`R × sᵢ` is a checked multiplication that returns `MathOverflow` rather than wrapping, and no
state is written when it trips. Overflow requires `R > i128::MAX / sᵢ`. With the largest
possible weight that is `R > 3.9 × 10²⁸`, unreachable for a Stellar Asset Contract, whose
classic assets cap at `i64::MAX ≈ 9.2 × 10¹⁸`. It is only theoretically reachable for a pure
`i128` Soroban token, which is why the check exists rather than an assumption. Keeping share
weights small (percentages, or basis points) keeps the ceiling astronomically far away.

## Storage and state archival

Soroban entries have a time-to-live and are archived when it lapses. Mismanaged TTL is a real
bug class, and for a splitter a specific one: **if a recipient's payout record expired while
the configuration survived, the missing record would read as `0` and that recipient could be
paid their entire entitlement a second time.**

Lumengate makes that impossible structurally. Every entry lives in **instance storage**:

| Key | Contents |
| --- | --- |
| `Config` | token, recipients, share weights, total shares — written once, never mutated |
| `Released` | `Map<Address, i128>`, cumulative payout per recipient |
| `TotalReleased` | `i128` |

Instance entries share a single TTL with the contract instance and its code, so they are
archived and restored **as one unit**. The configuration and the payout ledger cannot go out of
sync, because there is no state in which one exists and the other does not.

Every entry point extends the instance TTL back to the network maximum (`max_ttl()`) once it
has decayed 30 days below that maximum, so an active splitter never approaches archival.

The cost of this choice is that all state must fit in one ledger entry, which is why
**recipients are capped at 20**. That cap also keeps `release_all` — one token transfer per
recipient — inside the per-transaction resource limits.

If a splitter does sit unused long enough to be archived, it is restored the ordinary way, with
`stellar contract restore`; the accounting comes back intact.

## Security

**This contract is unaudited.** What follows is the reasoning behind its design, not a
guarantee.

**Authorization.** `release` and `release_all` deliberately require no authorization: anyone
may call them, including a third party paying the transaction fee on a recipient's behalf.
This is safe because a caller supplies only *which* recipient to pay — the destination address
is read from configuration written at deployment. There is no input by which a caller can
redirect funds. The token transfer is made from the splitter's own address, which it authorizes
implicitly.

**No admin.** There is no owner, no upgrade path and no withdrawal hatch. Nobody, including the
deployer, can change the recipients, change the shares, or remove funds by any route other than
paying recipients what they are owed. The trade-off is that a misconfigured deployment cannot be
fixed — verify the recipient list before deploying.

**Ordering.** Payout state is written before the token transfer (checks-effects-interactions).
The Soroban host forbids reentrancy today; the ordering costs nothing and holds if that changes.

**Validation at construction.** Duplicate recipients are rejected, since two entries for one
address would share a single payout record and under-pay it. Zero weights are rejected as
certainly unintended. The splitter's own address is rejected as a recipient, because paying
itself would recycle its balance into `R` and inflate every later entitlement.

### Assumptions and limits

- **One token per splitter, fixed at deployment.** A different token sent to the address is
  ignored by the contract and stranded there permanently. There is no sweep function.
- **The token must be well-behaved.** Payout math assumes a standard SEP-41 token that moves
  exactly the requested amount. A fee-on-transfer or rebasing token will not split correctly.
- **Clawback-enabled assets.** A Stellar asset with clawback lets its issuer remove tokens from
  the splitter directly. That lowers `R` beneath what has already been paid out. Payouts are
  clamped at zero so the contract cannot be driven into a negative transfer, but the recipients
  who had not yet claimed absorb the entire loss. Do not use a clawback-enabled asset unless
  that is acceptable.
- **Recipients are fixed forever.** See below.
- **Dust.** Up to one base unit per recipient may rest undistributed. See the dust rule.

## Scope

Deliberately excluded from this version, and good first contributions:

- **Mutable recipients** — adding, removing or reweighting after deployment, which needs an
  access-control model and careful handling of already-accrued entitlements.
- **Multiple tokens** — routing several tokens through one splitter.
- **Weighted governance** over configuration changes.
- **Chunked `release_all`** for recipient sets beyond the current cap of 20.

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[Apache-2.0](LICENSE).
