use soroban_sdk::{contracterror, contractevent, contracttype, Address, Vec};

/// Upper bound on the number of recipients accepted at construction.
///
/// The recipient set, the share weights and the per-recipient release ledger
/// all live in *instance* storage, which is a single ledger entry with a size
/// limit. The bound also keeps [`release_all`](crate::Lumengate::release_all)
/// — which performs one token transfer per recipient — comfortably inside the
/// per-transaction resource limits.
pub const MAX_RECIPIENTS: u32 = 20;

/// A recipient and its share weight, as supplied at construction.
///
/// `shares` is a relative weight, not a percentage: a recipient is owed
/// `shares / total_shares` of everything the contract has ever received.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Recipient {
    pub address: Address,
    pub shares: u32,
}

/// The immutable configuration written once by the constructor.
///
/// Nothing in this struct can change after deployment. Mutable payout
/// accounting is kept under [`DataKey::Released`] and
/// [`DataKey::TotalReleased`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    /// Address of the token (SAC or any SEP-41 token) this splitter routes.
    pub token: Address,
    /// Recipients in the order given at construction.
    pub recipients: Vec<Recipient>,
    /// Sum of every recipient's `shares`. Always `> 0`.
    pub total_shares: u32,
}

/// Instance-storage keys.
///
/// Every entry lives in instance storage so that configuration and payout
/// accounting share a single time-to-live and can never desynchronise: see the
/// storage notes in the README.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// [`Config`] — written once by the constructor.
    Config,
    /// `Map<Address, i128>` — cumulative amount already paid to each recipient.
    Released,
    /// `i128` — sum of the values in [`DataKey::Released`].
    TotalReleased,
}

/// Emitted once, when the contract is deployed and configured.
#[contractevent(topics = ["lumengate", "init"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Initialized {
    #[topic]
    pub token: Address,
    pub recipients: u32,
    pub total_shares: u32,
}

/// Emitted whenever funds are paid out to a recipient.
///
/// Not emitted when a release resolves to zero, so the event stream contains
/// only actual transfers.
#[contractevent(topics = ["lumengate", "release"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Release {
    #[topic]
    pub recipient: Address,
    pub amount: i128,
}

/// Errors returned by the contract.
///
/// Discriminants are part of the contract's public interface: never renumber an
/// existing variant, only append.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Construction was attempted with an empty recipient list.
    NoRecipients = 1,
    /// Construction was attempted with more than [`MAX_RECIPIENTS`] recipients.
    TooManyRecipients = 2,
    /// The same address appears twice in the recipient list.
    DuplicateRecipient = 3,
    /// A recipient was given a share weight of zero.
    ZeroShares = 4,
    /// A recipient is the splitter itself, which would trap funds in a cycle.
    RecipientIsSplitter = 5,
    /// The address is not one of the configured recipients.
    NotARecipient = 6,
    /// An arithmetic operation would overflow; no state was changed.
    MathOverflow = 7,
}
