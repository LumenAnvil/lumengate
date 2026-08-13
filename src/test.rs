#![cfg(test)]
extern crate std;

use soroban_sdk::{
    testutils::{storage::Instance as _, Address as _, Events as _, Ledger as _},
    token::{StellarAssetClient, TokenClient},
    vec, Address, Env, Event as _, Vec,
};

use crate::types::{Error, Initialized, Recipient, Release, MAX_RECIPIENTS};
use crate::{Lumengate, LumengateClient};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A splitter, the token it routes, and its recipients.
struct Splitter<'a> {
    env: Env,
    client: LumengateClient<'a>,
    token: TokenClient<'a>,
    minter: StellarAssetClient<'a>,
    recipients: Vec<Recipient>,
}

impl<'a> Splitter<'a> {
    /// Deploys a splitter over a fresh Stellar Asset Contract, one recipient per
    /// entry in `weights`.
    fn new(weights: &[u32]) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let issuer = Address::generate(&env);
        let asset = env.register_stellar_asset_contract_v2(issuer);
        let token_address = asset.address();

        let mut recipients = Vec::new(&env);
        for weight in weights {
            recipients.push_back(Recipient {
                address: Address::generate(&env),
                shares: *weight,
            });
        }

        let contract_id = env.register(Lumengate, (token_address.clone(), recipients.clone()));

        Splitter {
            client: LumengateClient::new(&env, &contract_id),
            token: TokenClient::new(&env, &token_address),
            minter: StellarAssetClient::new(&env, &token_address),
            recipients,
            env,
        }
    }

    /// Sends `amount` of the token to the splitter, as an external payer would.
    fn deposit(&self, amount: i128) {
        self.minter.mint(&self.client.address, &amount);
    }

    fn recipient(&self, index: u32) -> Address {
        self.recipients.get_unchecked(index).address
    }

    fn balance_of(&self, index: u32) -> i128 {
        self.token.balance(&self.recipient(index))
    }

    /// The token still sitting in the splitter.
    fn undistributed(&self) -> i128 {
        self.token.balance(&self.client.address)
    }

    /// Asserts the invariants that must hold after every operation.
    fn assert_invariants(&self) {
        let mut summed = 0i128;
        for recipient in self.recipients.iter() {
            let released = self.client.released(&recipient.address);
            assert!(released >= 0, "a recipient's released total went negative");
            summed += released;
        }

        assert_eq!(
            summed,
            self.client.total_released(),
            "per-recipient payouts must sum to total_released"
        );
        assert!(
            self.client.total_released() <= self.client.total_received(),
            "more was paid out than was ever received"
        );
        assert_eq!(
            self.client.total_received(),
            self.undistributed() + self.client.total_released(),
            "received must equal the balance still held plus everything paid out"
        );
        assert!(
            self.undistributed() >= 0,
            "the splitter must never be overdrawn"
        );
    }
}

/// Builds a recipient list with generated addresses and the given weights.
fn recipients_with(env: &Env, weights: &[u32]) -> Vec<Recipient> {
    let mut recipients = Vec::new(env);
    for weight in weights {
        recipients.push_back(Recipient {
            address: Address::generate(env),
            shares: *weight,
        });
    }
    recipients
}

fn fresh_token(env: &Env) -> Address {
    let issuer = Address::generate(env);
    env.register_stellar_asset_contract_v2(issuer).address()
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

#[test]
fn constructor_records_configuration() {
    let s = Splitter::new(&[1, 2, 3]);

    assert_eq!(s.client.total_shares(), 6);
    assert_eq!(s.client.recipients(), s.recipients);
    assert_eq!(s.client.token(), s.token.address);
    assert_eq!(s.client.total_released(), 0);
    assert_eq!(s.client.total_received(), 0);

    assert_eq!(s.client.shares(&s.recipient(0)), 1);
    assert_eq!(s.client.shares(&s.recipient(1)), 2);
    assert_eq!(s.client.shares(&s.recipient(2)), 3);
}

#[test]
fn constructor_accepts_the_maximum_recipient_count() {
    let weights = std::vec![1u32; MAX_RECIPIENTS as usize];
    let s = Splitter::new(&weights);

    assert_eq!(s.client.recipients().len(), MAX_RECIPIENTS);
    assert_eq!(s.client.total_shares(), MAX_RECIPIENTS);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn constructor_rejects_an_empty_recipient_list() {
    let env = Env::default();
    let token = fresh_token(&env);
    env.register(Lumengate, (token, Vec::<Recipient>::new(&env)));
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn constructor_rejects_too_many_recipients() {
    let env = Env::default();
    let token = fresh_token(&env);
    let weights = std::vec![1u32; MAX_RECIPIENTS as usize + 1];
    env.register(Lumengate, (token, recipients_with(&env, &weights)));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn constructor_rejects_a_duplicate_recipient() {
    let env = Env::default();
    let token = fresh_token(&env);
    let duplicated = Address::generate(&env);

    let recipients = vec![
        &env,
        Recipient {
            address: duplicated.clone(),
            shares: 1,
        },
        Recipient {
            address: duplicated,
            shares: 2,
        },
    ];

    env.register(Lumengate, (token, recipients));
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn constructor_rejects_a_zero_share_recipient() {
    let env = Env::default();
    let token = fresh_token(&env);
    env.register(Lumengate, (token, recipients_with(&env, &[1, 0])));
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn constructor_rejects_the_splitter_itself_as_a_recipient() {
    let env = Env::default();
    let token = fresh_token(&env);

    // Deploy once to learn a contract address, then redeploy at that same
    // address with the address itself in the recipient list.
    let address = env.register(Lumengate, (token.clone(), recipients_with(&env, &[1])));

    let recipients = vec![
        &env,
        Recipient {
            address: Address::generate(&env),
            shares: 1,
        },
        Recipient {
            address: address.clone(),
            shares: 1,
        },
    ];

    env.register_at(&address, Lumengate, (token, recipients));
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn constructor_rejects_share_weights_that_overflow() {
    let env = Env::default();
    let token = fresh_token(&env);
    env.register(
        Lumengate,
        (token, recipients_with(&env, &[u32::MAX, u32::MAX])),
    );
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

#[test]
fn views_report_zero_for_an_unknown_address() {
    let s = Splitter::new(&[1]);
    let stranger = Address::generate(&s.env);

    assert_eq!(s.client.shares(&stranger), 0);
    assert_eq!(s.client.released(&stranger), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn releasable_rejects_an_unknown_address() {
    let s = Splitter::new(&[1]);
    s.client.releasable(&Address::generate(&s.env));
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn release_rejects_an_unknown_address() {
    let s = Splitter::new(&[1]);
    s.client.release(&Address::generate(&s.env));
}

// ---------------------------------------------------------------------------
// Releasing
// ---------------------------------------------------------------------------

#[test]
fn release_pays_exactly_the_configured_share() {
    let s = Splitter::new(&[1, 3]);
    s.deposit(1_000);

    assert_eq!(s.client.releasable(&s.recipient(0)), 250);
    assert_eq!(s.client.releasable(&s.recipient(1)), 750);

    assert_eq!(s.client.release(&s.recipient(0)), 250);

    assert_eq!(s.balance_of(0), 250);
    assert_eq!(s.balance_of(1), 0);
    assert_eq!(s.undistributed(), 750);
    assert_eq!(s.client.released(&s.recipient(0)), 250);
    assert_eq!(s.client.total_released(), 250);
    assert_eq!(s.client.total_received(), 1_000);
    s.assert_invariants();
}

#[test]
fn a_second_release_pays_nothing() {
    let s = Splitter::new(&[1, 1]);
    s.deposit(1_000);

    assert_eq!(s.client.release(&s.recipient(0)), 500);
    assert_eq!(s.client.release(&s.recipient(0)), 0);
    assert_eq!(s.client.release(&s.recipient(0)), 0);

    assert_eq!(s.balance_of(0), 500);
    assert_eq!(s.client.released(&s.recipient(0)), 500);
    assert_eq!(s.client.total_released(), 500);
    s.assert_invariants();
}

#[test]
fn release_without_any_funds_is_a_no_op() {
    let s = Splitter::new(&[1, 1]);

    assert_eq!(s.client.releasable(&s.recipient(0)), 0);
    assert_eq!(s.client.release(&s.recipient(0)), 0);
    assert_eq!(s.balance_of(0), 0);
    assert_eq!(s.client.total_released(), 0);
    s.assert_invariants();
}

#[test]
fn release_all_pays_every_recipient() {
    let s = Splitter::new(&[1, 2, 7]);
    s.deposit(10_000);

    assert_eq!(s.client.release_all(), 10_000);

    assert_eq!(s.balance_of(0), 1_000);
    assert_eq!(s.balance_of(1), 2_000);
    assert_eq!(s.balance_of(2), 7_000);
    assert_eq!(s.undistributed(), 0);
    assert_eq!(s.client.total_released(), 10_000);
    s.assert_invariants();
}

#[test]
fn release_all_is_idempotent() {
    let s = Splitter::new(&[1, 2, 7]);
    s.deposit(10_000);

    assert_eq!(s.client.release_all(), 10_000);
    assert_eq!(s.client.release_all(), 0);
    assert_eq!(s.client.release_all(), 0);

    assert_eq!(s.balance_of(0), 1_000);
    assert_eq!(s.client.total_released(), 10_000);
    s.assert_invariants();
}

#[test]
fn release_all_skips_recipients_already_paid() {
    let s = Splitter::new(&[1, 1, 2]);
    s.deposit(1_000);

    assert_eq!(s.client.release(&s.recipient(2)), 500);
    // Only the two unpaid recipients are left to settle.
    assert_eq!(s.client.release_all(), 500);

    assert_eq!(s.balance_of(0), 250);
    assert_eq!(s.balance_of(1), 250);
    assert_eq!(s.balance_of(2), 500);
    s.assert_invariants();
}

// ---------------------------------------------------------------------------
// Rounding and dust
// ---------------------------------------------------------------------------

#[test]
fn indivisible_amounts_leave_dust_in_the_splitter() {
    let s = Splitter::new(&[1, 1, 1]);
    s.deposit(100);

    assert_eq!(s.client.release_all(), 99);

    // floor(100 / 3) each, and the odd unit stays put.
    assert_eq!(s.balance_of(0), 33);
    assert_eq!(s.balance_of(1), 33);
    assert_eq!(s.balance_of(2), 33);
    assert_eq!(s.undistributed(), 1);
    s.assert_invariants();
}

#[test]
fn dust_is_paid_out_once_more_funds_arrive() {
    let s = Splitter::new(&[1, 1, 1]);

    s.deposit(100);
    s.client.release_all();
    assert_eq!(s.undistributed(), 1);

    // Entitlements are computed on the cumulative total, so the unit held back
    // is folded into the next distribution rather than lost.
    s.deposit(200);
    assert_eq!(s.client.total_received(), 300);
    assert_eq!(s.client.release_all(), 201);

    assert_eq!(s.balance_of(0), 100);
    assert_eq!(s.balance_of(1), 100);
    assert_eq!(s.balance_of(2), 100);
    assert_eq!(s.undistributed(), 0);
    s.assert_invariants();
}

#[test]
fn dust_never_exceeds_one_unit_per_recipient() {
    let s = Splitter::new(&[1, 2, 3]);
    s.deposit(100);

    assert_eq!(s.client.release_all(), 99);

    // floor of 100/6, 200/6, 300/6.
    assert_eq!(s.balance_of(0), 16);
    assert_eq!(s.balance_of(1), 33);
    assert_eq!(s.balance_of(2), 50);
    assert!(s.undistributed() < s.recipients.len() as i128);
    s.assert_invariants();
}

#[test]
fn payouts_do_not_depend_on_release_order() {
    let forward = Splitter::new(&[5, 3, 1]);
    forward.deposit(1_000);
    forward.client.release(&forward.recipient(0));
    forward.client.release(&forward.recipient(1));
    forward.client.release(&forward.recipient(2));

    let reverse = Splitter::new(&[5, 3, 1]);
    reverse.deposit(1_000);
    reverse.client.release(&reverse.recipient(2));
    reverse.client.release(&reverse.recipient(1));
    reverse.client.release(&reverse.recipient(0));

    for i in 0..3 {
        assert_eq!(forward.balance_of(i), reverse.balance_of(i));
    }
    assert_eq!(forward.undistributed(), reverse.undistributed());
    forward.assert_invariants();
    reverse.assert_invariants();
}

// ---------------------------------------------------------------------------
// Repeated and interleaved activity
// ---------------------------------------------------------------------------

#[test]
fn deposits_interleaved_with_releases_stay_correct() {
    let s = Splitter::new(&[1, 1]);

    s.deposit(100);
    assert_eq!(s.client.release(&s.recipient(0)), 50);

    s.deposit(100);
    // Recipient 0 is owed half of the second deposit only.
    assert_eq!(s.client.releasable(&s.recipient(0)), 50);
    // Recipient 1 has never been paid, so is owed half of everything.
    assert_eq!(s.client.releasable(&s.recipient(1)), 100);

    assert_eq!(s.client.release(&s.recipient(0)), 50);
    assert_eq!(s.client.release(&s.recipient(1)), 100);

    assert_eq!(s.balance_of(0), 100);
    assert_eq!(s.balance_of(1), 100);
    assert_eq!(s.undistributed(), 0);
    s.assert_invariants();
}

#[test]
fn many_small_deposits_conserve_every_unit() {
    let s = Splitter::new(&[3, 1]);
    let mut deposited = 0i128;

    for round in 1..=25i128 {
        s.deposit(round);
        deposited += round;

        if round % 3 == 0 {
            s.client.release(&s.recipient(0));
        }
        if round % 5 == 0 {
            s.client.release_all();
        }
        s.assert_invariants();
    }

    s.client.release_all();

    // Nothing is created or destroyed: everything deposited is either paid out
    // or still held as dust.
    assert_eq!(
        s.balance_of(0) + s.balance_of(1) + s.undistributed(),
        deposited
    );
    assert_eq!(s.client.total_received(), deposited);
    assert!(s.undistributed() < 2);
    s.assert_invariants();
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

#[test]
fn anyone_can_trigger_a_release_and_funds_reach_the_recipient() {
    let s = Splitter::new(&[1, 1]);
    s.deposit(1_000);

    // Withdraw every mocked authorization: a release must still succeed, which
    // is what makes the contract callable by a third party paying the fee.
    s.env.set_auths(&[]);

    assert_eq!(s.client.release(&s.recipient(0)), 500);
    assert_eq!(s.balance_of(0), 500);
    // The caller cannot divert anything: the destination came from storage.
    assert_eq!(s.balance_of(1), 0);
    s.assert_invariants();
}

#[test]
fn a_release_never_pays_an_address_of_the_callers_choosing() {
    let s = Splitter::new(&[1, 1]);
    s.deposit(1_000);

    let attacker = Address::generate(&s.env);
    s.client.release(&s.recipient(0));

    assert_eq!(s.token.balance(&attacker), 0);
    assert_eq!(s.balance_of(0), 500);
    s.assert_invariants();
}

// ---------------------------------------------------------------------------
// Storage lifetime
// ---------------------------------------------------------------------------

#[test]
fn operations_extend_the_instance_lifetime() {
    let s = Splitter::new(&[1, 1]);
    s.deposit(1_000);

    let max_ttl = s.env.storage().max_ttl();

    // Let the instance age, then confirm a release restores it to the maximum.
    s.env.ledger().with_mut(|li| {
        li.sequence_number += max_ttl / 2;
    });

    s.client.release(&s.recipient(0));

    s.env.as_contract(&s.client.address, || {
        assert_eq!(s.env.storage().instance().get_ttl(), max_ttl);
    });
}

#[test]
fn state_survives_a_long_gap_between_releases() {
    let s = Splitter::new(&[2, 1]);
    s.deposit(300);
    assert_eq!(s.client.release(&s.recipient(0)), 200);

    s.env.ledger().with_mut(|li| {
        li.sequence_number += 100_000;
    });

    // Accounting is intact, so the paid recipient is still owed nothing.
    assert_eq!(s.client.released(&s.recipient(0)), 200);
    assert_eq!(s.client.release(&s.recipient(0)), 0);
    assert_eq!(s.client.release(&s.recipient(1)), 100);
    s.assert_invariants();
}

// ---------------------------------------------------------------------------
// Arithmetic limits
// ---------------------------------------------------------------------------

#[test]
fn very_large_balances_are_handled_without_wrapping() {
    let s = Splitter::new(&[1, 1]);
    let huge = i128::MAX / 4;
    s.deposit(huge);

    assert_eq!(s.client.release(&s.recipient(0)), huge / 2);
    assert_eq!(s.balance_of(0), huge / 2);
    s.assert_invariants();
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn an_entitlement_that_would_overflow_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let issuer = Address::generate(&env);
    let token_address = env.register_stellar_asset_contract_v2(issuer).address();

    // Weights large enough that `received × shares` leaves the i128 range.
    let recipients = recipients_with(&env, &[u32::MAX / 2, u32::MAX / 2]);
    let contract_id = env.register(Lumengate, (token_address.clone(), recipients.clone()));

    StellarAssetClient::new(&env, &token_address).mint(&contract_id, &i128::MAX);

    // Confirm the balance really is in place, so the panic below can only come
    // from the entitlement arithmetic and not from the mint.
    let client = LumengateClient::new(&env, &contract_id);
    assert_eq!(
        TokenClient::new(&env, &token_address).balance(&contract_id),
        i128::MAX
    );
    assert_eq!(
        client.try_releasable(&recipients.get_unchecked(0).address),
        Err(Ok(Error::MathOverflow))
    );

    client.release(&recipients.get_unchecked(0).address);
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[test]
fn deployment_emits_an_initialization_event() {
    let env = Env::default();
    let token = fresh_token(&env);
    let recipients = recipients_with(&env, &[1, 3]);
    let contract_id = env.register(Lumengate, (token.clone(), recipients));

    let expected = Initialized {
        token,
        recipients: 2,
        total_shares: 4,
    };

    assert_eq!(
        env.events().all().filter_by_contract(&contract_id),
        std::vec![expected.to_xdr(&env, &contract_id)],
    );
}

#[test]
fn a_release_emits_an_event_carrying_the_amount() {
    let s = Splitter::new(&[1, 1]);
    s.deposit(1_000);
    s.client.release(&s.recipient(0));

    let expected = Release {
        recipient: s.recipient(0),
        amount: 500,
    };

    assert_eq!(
        s.env.events().all().filter_by_contract(&s.client.address),
        std::vec![expected.to_xdr(&s.env, &s.client.address)],
    );
}

#[test]
fn a_release_that_pays_nothing_emits_no_event() {
    let s = Splitter::new(&[1, 1]);
    s.deposit(1_000);
    s.client.release(&s.recipient(0));

    // A second release moves nothing, so it must not report a transfer.
    assert_eq!(s.client.release(&s.recipient(0)), 0);
    assert_eq!(
        s.env.events().all().filter_by_contract(&s.client.address),
        std::vec![],
    );
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

#[test]
fn full_lifecycle() {
    // A three-way split: 50% / 30% / 20%.
    let s = Splitter::new(&[50, 30, 20]);
    assert_eq!(s.client.total_shares(), 100);

    // Nothing has arrived yet.
    assert_eq!(s.client.release_all(), 0);
    s.assert_invariants();

    // First revenue arrives and one recipient cashes out early.
    s.deposit(10_000);
    assert_eq!(s.client.releasable(&s.recipient(0)), 5_000);
    assert_eq!(s.client.release(&s.recipient(0)), 5_000);
    assert_eq!(s.undistributed(), 5_000);
    s.assert_invariants();

    // More revenue arrives before the others have claimed anything.
    s.deposit(10_000);
    assert_eq!(s.client.total_received(), 20_000);
    assert_eq!(s.client.releasable(&s.recipient(0)), 5_000);
    assert_eq!(s.client.releasable(&s.recipient(1)), 6_000);
    assert_eq!(s.client.releasable(&s.recipient(2)), 4_000);

    // Everyone settles up.
    assert_eq!(s.client.release_all(), 15_000);
    assert_eq!(s.balance_of(0), 10_000);
    assert_eq!(s.balance_of(1), 6_000);
    assert_eq!(s.balance_of(2), 4_000);
    assert_eq!(s.undistributed(), 0);
    assert_eq!(s.client.total_released(), 20_000);
    s.assert_invariants();

    // A long quiet period, then a final indivisible deposit.
    s.env.ledger().with_mut(|li| {
        li.sequence_number += 50_000;
    });
    s.deposit(7);
    assert_eq!(s.client.release_all(), 6);

    assert_eq!(s.balance_of(0), 10_003);
    assert_eq!(s.balance_of(1), 6_002);
    assert_eq!(s.balance_of(2), 4_001);
    assert_eq!(s.undistributed(), 1);
    assert_eq!(s.client.total_received(), 20_007);
    s.assert_invariants();

    // Repeating everything changes nothing.
    assert_eq!(s.client.release_all(), 0);
    s.assert_invariants();
}
