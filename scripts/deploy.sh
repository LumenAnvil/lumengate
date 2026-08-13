#!/usr/bin/env bash
#
# End-to-end example: build a splitter, deploy it, fund it, and pay everyone.
#
# Runs against testnet by default and creates throwaway accounts, so it is safe
# to run repeatedly. Requires stellar-cli and the wasm32v1-none target.
#
#   ./scripts/deploy.sh
#   NETWORK=local ./scripts/deploy.sh
#
set -euo pipefail

NETWORK="${NETWORK:-testnet}"
WASM="target/wasm32v1-none/release/lumengate.wasm"

# Share weights below are 50 / 30 / 20, i.e. percentages. Any integer weights
# work; only their ratio matters.
ALICE_SHARES=50
BOB_SHARES=30
CAROL_SHARES=20

# Amount of the test asset to route through the splitter, in stroops.
DEPOSIT=10000000   # 1.0 units at 7 decimals

say() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

# ---------------------------------------------------------------------------

say "Building"
stellar contract build
ls -l "$WASM"

say "Creating accounts on $NETWORK"
for who in deployer issuer alice bob carol payer; do
  stellar keys generate "lg-$who" --network "$NETWORK" --fund --overwrite
done

ALICE=$(stellar keys address lg-alice)
BOB=$(stellar keys address lg-bob)
CAROL=$(stellar keys address lg-carol)
ISSUER=$(stellar keys address lg-issuer)

echo "alice: $ALICE"
echo "bob:   $BOB"
echo "carol: $CAROL"

# ---------------------------------------------------------------------------
# A Stellar Asset Contract to route. Any SEP-41 token works just as well; this
# script issues its own so the example is self-contained.
# ---------------------------------------------------------------------------

say "Deploying the test asset LMNG:$ISSUER"
TOKEN=$(stellar contract asset deploy \
  --asset "LMNG:$ISSUER" \
  --source-account lg-issuer \
  --network "$NETWORK")
echo "token: $TOKEN"

# ---------------------------------------------------------------------------
# Deploy the splitter. Configuration is passed to __constructor after the `--`
# separator, so deployment and setup are one atomic operation.
# ---------------------------------------------------------------------------

say "Deploying the splitter"
SPLITTER=$(stellar contract deploy \
  --wasm "$WASM" \
  --source-account lg-deployer \
  --network "$NETWORK" \
  -- \
  --token "$TOKEN" \
  --recipients "[
     {\"address\":\"$ALICE\",\"shares\":$ALICE_SHARES},
     {\"address\":\"$BOB\",\"shares\":$BOB_SHARES},
     {\"address\":\"$CAROL\",\"shares\":$CAROL_SHARES}
   ]")
echo "splitter: $SPLITTER"

say "Configuration as deployed"
stellar contract invoke --id "$SPLITTER" --source-account lg-payer --network "$NETWORK" \
  -- recipients
stellar contract invoke --id "$SPLITTER" --source-account lg-payer --network "$NETWORK" \
  -- total_shares

# ---------------------------------------------------------------------------
# Fund it. There is no deposit function: an ordinary transfer to the splitter's
# address is all it takes.
# ---------------------------------------------------------------------------

say "Sending $DEPOSIT stroops to the splitter"
stellar contract invoke --id "$TOKEN" --source-account lg-issuer --network "$NETWORK" \
  -- mint --to "$SPLITTER" --amount "$DEPOSIT"

stellar contract invoke --id "$SPLITTER" --source-account lg-payer --network "$NETWORK" \
  -- total_received

say "Owed to each recipient"
for who in "$ALICE" "$BOB" "$CAROL"; do
  printf '%s: ' "$who"
  stellar contract invoke --id "$SPLITTER" --source-account lg-payer --network "$NETWORK" \
    -- releasable --who "$who"
done

# ---------------------------------------------------------------------------
# Release. Note the source account: `lg-payer` is an unrelated third party with
# no share in the splitter. It pays the transaction fee; the funds still go to
# the configured recipients and nowhere else.
# ---------------------------------------------------------------------------

say "Releasing to alice, called by an unrelated account"
stellar contract invoke --id "$SPLITTER" --source-account lg-payer --network "$NETWORK" \
  -- release --who "$ALICE"

say "Releasing to everyone else"
stellar contract invoke --id "$SPLITTER" --source-account lg-payer --network "$NETWORK" \
  -- release_all

say "Final balances"
for who in "$ALICE" "$BOB" "$CAROL"; do
  printf '%s: ' "$who"
  stellar contract invoke --id "$TOKEN" --source-account lg-payer --network "$NETWORK" \
    -- balance --id "$who"
done

printf 'still held by the splitter (dust): '
stellar contract invoke --id "$TOKEN" --source-account lg-payer --network "$NETWORK" \
  -- balance --id "$SPLITTER"

say "Calling release_all again is a no-op"
stellar contract invoke --id "$SPLITTER" --source-account lg-payer --network "$NETWORK" \
  -- release_all

say "Done. Splitter: $SPLITTER"
