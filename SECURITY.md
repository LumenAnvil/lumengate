# Security policy

Lumengate is a smart contract that routes funds. A vulnerability can cause
irreversible loss, so please treat reports accordingly.

## Reporting a vulnerability

**Do not open a public issue for a security problem.** Instead, open a private
report through GitHub Security Advisories:

<https://github.com/LumenAnvil/lumengate/security/advisories/new>

Please include:

- a description of the issue and its impact (e.g. funds reaching an address that
  is not a configured recipient, or the splitter being overdrawn),
- the smallest reproduction you can manage — ideally a failing test, or the
  exact `stellar contract invoke` calls and the configuration they ran against,
- the commit or deployed contract you observed it on.

You can expect an acknowledgement, and we will keep you updated as we work on a
fix. Once a fix is released, we are glad to credit you unless you prefer
otherwise.

## Scope

The contract code under `src/` and the invariants it is meant to uphold:

- funds can only ever reach the recipients fixed at deployment;
- the sum released to recipients never exceeds what the contract received;
- a recipient can never be paid more than their share entitles them to.

Deployment scripts and documentation are in scope for issues that would lead a
user to deploy or operate the contract unsafely.
