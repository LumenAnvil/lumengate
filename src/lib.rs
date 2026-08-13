#![no_std]

pub mod types;

#[cfg(test)]
mod test;

use soroban_sdk::{contract, contractimpl, contractmeta, token, Address, Env, Map, Vec};

use crate::types::{Config, DataKey, Error, Initialized, Recipient, Release, MAX_RECIPIENTS};

contractmeta!(key = "name", val = "lumengate");
contractmeta!(
    key = "desc",
    val = "Fixed-share payment splitter for a single token."
);

/// Ledgers in roughly one day, assuming ~5 second ledger close times.
const DAY_IN_LEDGERS: u32 = 17_280;

/// How far the instance TTL is allowed to decay before a call re-extends it.
///
/// Every entry-point extends the instance to the network maximum once its
/// remaining TTL drops more than this far below that maximum, which keeps the
/// number of TTL writes low while leaving a wide margin against archival.
const TTL_BUMP_INTERVAL: u32 = 30 * DAY_IN_LEDGERS;

#[contract]
pub struct Lumengate;

// ---------------------------------------------------------------------------
// Storage helpers
//
// All state lives in instance storage: see the "Storage and state archival"
// section of the README for why configuration and payout accounting must share
// a single lifetime.
// ---------------------------------------------------------------------------

/// Extends the instance (and contract code) TTL back to the network maximum.
fn extend_instance_ttl(env: &Env) {
    let max_ttl = env.storage().max_ttl();
    let threshold = max_ttl.saturating_sub(TTL_BUMP_INTERVAL);
    env.storage().instance().extend_ttl(threshold, max_ttl);
}

/// Reads the immutable configuration.
///
/// The constructor writes this before any other entry point can run, so the
/// entry is always present.
fn read_config(env: &Env) -> Config {
    env.storage().instance().get(&DataKey::Config).unwrap()
}

/// Reads the cumulative-payout ledger.
fn read_released(env: &Env) -> Map<Address, i128> {
    env.storage()
        .instance()
        .get(&DataKey::Released)
        .unwrap_or_else(|| Map::new(env))
}

/// Reads the amount already paid to a single address.
fn read_released_for(env: &Env, who: &Address) -> i128 {
    read_released(env).get(who.clone()).unwrap_or(0)
}

/// Reads the sum of all payouts made so far.
fn read_total_released(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&DataKey::TotalReleased)
        .unwrap_or(0)
}

/// Returns the share weight configured for `who`, or `None` if `who` is not a
/// recipient.
fn shares_of(config: &Config, who: &Address) -> Option<u32> {
    for recipient in config.recipients.iter() {
        if &recipient.address == who {
            return Some(recipient.shares);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Distribution math
//
// Writing `R` for everything the splitter has ever received, `sᵢ` for a
// recipient's share weight and `S` for the sum of all weights:
//
//     R  = balance(splitter) + total_released
//     Eᵢ = floor(R × sᵢ / S)          cumulative entitlement
//     owedᵢ = Eᵢ − releasedᵢ          paid out by the next release
//
// `R` never decreases: a release lowers the balance and raises `total_released`
// by the same amount, so only an incoming transfer moves it. `Eᵢ` is therefore
// non-decreasing and `owedᵢ` is never negative. Because `Σ Eᵢ ≤ R`, the balance
// always covers what every recipient is owed. See the README for the full
// argument and for the dust rule.
// ---------------------------------------------------------------------------

/// Everything the splitter has ever received, paid out or not.
fn total_received(env: &Env, config: &Config) -> Result<i128, Error> {
    let balance =
        token::TokenClient::new(env, &config.token).balance(&env.current_contract_address());
    balance
        .checked_add(read_total_released(env))
        .ok_or(Error::MathOverflow)
}

/// The cumulative entitlement of a recipient holding `shares`, given `received`.
///
/// Floor division: the remainder stays in the contract and is folded into
/// everyone's entitlement as soon as more funds arrive.
fn entitlement(received: i128, shares: u32, total_shares: u32) -> Result<i128, Error> {
    received
        .checked_mul(i128::from(shares))
        .ok_or(Error::MathOverflow)
        .map(|scaled| scaled / i128::from(total_shares))
}

/// Computes what `who` is owed right now, without moving anything.
fn compute_owed(env: &Env, config: &Config, who: &Address) -> Result<i128, Error> {
    let shares = shares_of(config, who).ok_or(Error::NotARecipient)?;
    let received = total_received(env, config)?;
    let earned = entitlement(received, shares, config.total_shares)?;
    let owed = earned - read_released_for(env, who);

    // `earned` is normally at least what has already been paid, because `R` only
    // grows. A token whose balance can fall without the splitter transferring —
    // a clawback-enabled Stellar asset, say — breaks that assumption, so clamp
    // rather than hand a negative amount to the token contract.
    Ok(if owed > 0 { owed } else { 0 })
}

/// Records a payout and transfers it.
///
/// State is written before the token call, so a recipient's ledger is already
/// updated by the time control leaves this contract.
fn pay(env: &Env, config: &Config, who: &Address, amount: i128) -> Result<(), Error> {
    let mut released = read_released(env);
    let already = released.get(who.clone()).unwrap_or(0);
    released.set(
        who.clone(),
        already.checked_add(amount).ok_or(Error::MathOverflow)?,
    );

    let total = read_total_released(env)
        .checked_add(amount)
        .ok_or(Error::MathOverflow)?;

    env.storage().instance().set(&DataKey::Released, &released);
    env.storage()
        .instance()
        .set(&DataKey::TotalReleased, &total);

    token::TokenClient::new(env, &config.token).transfer(
        &env.current_contract_address(),
        who,
        &amount,
    );

    Release {
        recipient: who.clone(),
        amount,
    }
    .publish(env);

    Ok(())
}

#[contractimpl]
impl Lumengate {
    /// Configures the splitter at deploy time.
    ///
    /// Runs exactly once, atomically with deployment: the Soroban host is the
    /// only caller permitted to invoke it, so there is no window in which an
    /// unconfigured contract is visible on-chain and nothing to re-initialize.
    ///
    /// `token` is the address of the token this splitter routes — a Stellar
    /// Asset Contract or any SEP-41 token. `recipients` are the payees and
    /// their relative share weights, fixed for the life of the contract.
    ///
    /// # Errors
    ///
    /// - [`Error::NoRecipients`] if `recipients` is empty.
    /// - [`Error::TooManyRecipients`] if it holds more than [`MAX_RECIPIENTS`].
    /// - [`Error::ZeroShares`] if any recipient has a weight of zero, which
    ///   would entitle it to nothing.
    /// - [`Error::DuplicateRecipient`] if an address appears twice; the two
    ///   entries would share one payout ledger and under-pay the address.
    /// - [`Error::RecipientIsSplitter`] if a recipient is the splitter itself,
    ///   which would recycle its own balance and inflate every later payout.
    /// - [`Error::MathOverflow`] if the share weights sum beyond `u32`.
    pub fn __constructor(
        env: Env,
        token: Address,
        recipients: Vec<Recipient>,
    ) -> Result<(), Error> {
        let count = recipients.len();
        if count == 0 {
            return Err(Error::NoRecipients);
        }
        if count > MAX_RECIPIENTS {
            return Err(Error::TooManyRecipients);
        }

        let splitter = env.current_contract_address();
        let mut total_shares: u32 = 0;

        for i in 0..count {
            let recipient = recipients.get_unchecked(i);

            if recipient.shares == 0 {
                return Err(Error::ZeroShares);
            }
            if recipient.address == splitter {
                return Err(Error::RecipientIsSplitter);
            }
            for j in (i + 1)..count {
                if recipients.get_unchecked(j).address == recipient.address {
                    return Err(Error::DuplicateRecipient);
                }
            }

            total_shares = total_shares
                .checked_add(recipient.shares)
                .ok_or(Error::MathOverflow)?;
        }

        let config = Config {
            token: token.clone(),
            recipients,
            total_shares,
        };
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage()
            .instance()
            .set(&DataKey::Released, &Map::<Address, i128>::new(&env));
        env.storage()
            .instance()
            .set(&DataKey::TotalReleased, &0i128);
        extend_instance_ttl(&env);

        Initialized {
            token,
            recipients: count,
            total_shares,
        }
        .publish(&env);

        Ok(())
    }

    /// The token this splitter routes.
    pub fn token(env: Env) -> Address {
        read_config(&env).token
    }

    /// The configured recipients and their share weights, in construction order.
    pub fn recipients(env: Env) -> Vec<Recipient> {
        read_config(&env).recipients
    }

    /// The sum of all configured share weights.
    pub fn total_shares(env: Env) -> u32 {
        read_config(&env).total_shares
    }

    /// The share weight of `who`, or `0` if `who` is not a recipient.
    pub fn shares(env: Env, who: Address) -> u32 {
        shares_of(&read_config(&env), &who).unwrap_or(0)
    }

    /// The total amount already paid to `who`, or `0` if `who` is not a
    /// recipient.
    pub fn released(env: Env, who: Address) -> i128 {
        read_released_for(&env, &who)
    }

    /// The total amount paid out to all recipients so far.
    pub fn total_released(env: Env) -> i128 {
        read_total_released(&env)
    }

    /// Everything the splitter has ever received, whether or not it has been
    /// paid out: its current balance plus everything already released.
    pub fn total_received(env: Env) -> Result<i128, Error> {
        let config = read_config(&env);
        total_received(&env, &config)
    }

    /// The amount `who` would receive from [`Lumengate::release`] right now.
    ///
    /// # Errors
    ///
    /// [`Error::NotARecipient`] if `who` is not a configured recipient.
    pub fn releasable(env: Env, who: Address) -> Result<i128, Error> {
        let config = read_config(&env);
        compute_owed(&env, &config, &who)
    }

    /// Pays `who` everything they are owed and returns the amount transferred.
    ///
    /// Permissionless by design: anyone may call this, including a third party
    /// paying the fee on a recipient's behalf. It is safe because the
    /// destination comes from the configuration written at deployment, never
    /// from the caller — a caller chooses only *which* recipient is paid, never
    /// where the funds go.
    ///
    /// Returns `0` and transfers nothing when nothing is owed, so calling it
    /// repeatedly is harmless and can never pay twice.
    ///
    /// # Errors
    ///
    /// - [`Error::NotARecipient`] if `who` is not a configured recipient.
    /// - [`Error::MathOverflow`] if the payout arithmetic would overflow, in
    ///   which case no state is changed and nothing is transferred.
    pub fn release(env: Env, who: Address) -> Result<i128, Error> {
        extend_instance_ttl(&env);

        let config = read_config(&env);
        let owed = compute_owed(&env, &config, &who)?;
        if owed == 0 {
            return Ok(0);
        }

        pay(&env, &config, &who, owed)?;
        Ok(owed)
    }

    /// Pays every recipient what they are owed and returns the total moved.
    ///
    /// Equivalent to calling [`Lumengate::release`] once per recipient, and
    /// bounded by [`MAX_RECIPIENTS`] so it stays inside per-transaction
    /// resource limits. Recipients owed nothing are skipped rather than
    /// aborting the batch.
    ///
    /// The order recipients are paid in does not affect what any of them
    /// receives: paying one lowers the balance and raises `total_released`
    /// equally, leaving the total received — and so everyone's entitlement —
    /// unchanged.
    pub fn release_all(env: Env) -> Result<i128, Error> {
        extend_instance_ttl(&env);

        let config = read_config(&env);
        let mut distributed: i128 = 0;

        for recipient in config.recipients.iter() {
            let owed = compute_owed(&env, &config, &recipient.address)?;
            if owed == 0 {
                continue;
            }
            pay(&env, &config, &recipient.address, owed)?;
            distributed = distributed.checked_add(owed).ok_or(Error::MathOverflow)?;
        }

        Ok(distributed)
    }
}
