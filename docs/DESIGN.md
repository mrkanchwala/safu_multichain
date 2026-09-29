# SAFU Protection Pool on Stellar: design

## The problem

Two things happen to crypto wallets and positions that no existing product covers together: a wallet gets drained by phishing, an approval exploit, or a stolen key; or a lending position gets liquidated on a price that was never real. Both are losses a user did nothing wrong to suffer, and both are usually met with either nothing, or a discretionary compensation decision taken by a committee weeks later.

SAFU pays for both, automatically, from one pool. Stake USDC once; it covers a wallet drain on Ethereum, Solana or Stellar, or a wrongful liquidation on any lending market, at a deterministic multiple of your own stake. No vote, no appeal, no human decides.

## One pool, not two contracts

This build forks SAFU's own live, audited protection-pool, the same contract design already running on Stellar mainnet, rather than building a new lending market. That choice follows directly from a simple fact: `submit_claim` doesn't care *why* a loss happened. It checks an oracle signature, a tier ceiling, and a real-loss cap. Wrongful liquidation slots into that same claim path as a second detected cause alongside wallet drain, it needed a new off-chain detector, not a new contract.

The pool is USDC-denominated. Whatever asset was lost, ETH, USDC, USDT on Ethereum, SOL on Solana, XLM on Stellar, the payout always settles in USDC from this one pool on Stellar. Stellar holds the money and the price truth for every covered chain (see "Pricing" below).

## Coverage, by tier

A wallet's on-chain history sets its tier, computed off-chain from age and activity, older, quieter wallets score higher. Tier sets a coverage *ceiling*, not a fixed payout:

| Tier | Ceiling |
|---|---|
| A | 15x stake |
| B | 10x stake |
| C | 5x stake |

The actual entitlement is `min(ceiling, real loss)`, capped by the ceiling but never invented above it. Yield later credited to a stake never raises this ceiling, it's anchored to the fixed amount admitted at stake time, which the contract already bounds to a small fraction of the pool cap. Staking is permissionless; anyone can stake any amount inside the bounds, no whitelist.

## Ownership: who can claim what

A staker registers up to three wallets they want covered, on any mix of chains, **before** anything happens to them. A claim can only name a wallet already on that list, checked against a timestamp that must predate the loss.

This closes a real gap that a naive multi-chain design opens: without it, a stranger's public drain transaction could be cited by anyone as their own loss. Pre-registration means the binding exists before the loss does, so citing someone else's drain afterward gains nothing. It does not require a signature from the drained wallet itself, deliberately, since key compromise is a covered vector, and demanding a signature from a compromised wallet would authenticate the thief, not the victim.

## Pricing

Every covered asset is priced through Reflector, the Stellar-native oracle network, read from its mainnet feed regardless of which chain the loss happened on. The price used for a payout is captured at the moment the loss is detected, not looked up later when the claim is filed, since a filing can happen up to 30 days after the fact and Reflector's own retention window is far shorter than that. Capturing early and storing the result makes the entitlement both correct at the time and auditable afterward.

## Wrongful liquidation

A lending position liquidated on a manufactured price is a covered loss, on any of the three chains. The detector compares the price a liquidation was executed at against Reflector's own price history over a window ending at that liquidation, never against the lending market's own feed, which is exactly the tick under question. A liquidation priced inside that comparison is a genuine market move and pays nothing; one priced meaningfully outside it is treated as fed a manufactured price, and the claim path opens.

Self-liquidation needs no special handling: the check evaluates the price, never who triggered the liquidation. If a lending market really was fed a bad price, the loss is real regardless of who benefits from reporting it, that is the correct outcome, not a gap.

The exact comparison window and threshold are not published here, for the same reason SAFU's fraud-scoring internals never are: publishing the exact numbers makes the check gameable. The structural mechanism, an independent price history, compared over a window, with a flag on excess deviation, is the part that matters and the part that's disclosed.

## On-chain and off-chain

| On-chain (audited, frozen) | Off-chain (Python, changeable) |
|---|---|
| Tier ceilings (15x / 10x / 5x) | Which wallets are registered to which staker |
| `entitlement > ceiling` rejection | The wrongful-liquidation price check |
| Oracle signature verification | Reflector price reads |
| 90-day time gate, cooldown, vesting | The actual entitlement number |
| Solvency and daily outflow caps | Real-loss capping |

The contract never learns what was lost, on which chain, or why the claim was ruled wrongful, it verifies a signed number against a ceiling. Everything that decides *that* number is off-chain and can be corrected without touching the audited artifact.

## Yield

Staked USDC deploys into a DeFindex vault targeting Blend's USDC pool. Realised yield splits 50/50: half compounds into every live staker's own withdrawable balance (principal plus their proportional share, computed via a growing index rather than per-user share tokens, the index approach avoids a known share-price manipulation pattern that a naive share-token design would be exposed to), and half becomes the protocol's own, withdrawable on its own schedule. Neither half auto-transfers the moment it's realised.

## What is deliberately not covered

Two specific gaps in the underlying scanner, disclosed rather than hidden: USDC and USDT losses on Solana (no SPL balance-delta path yet), and USDC losses on Stellar itself (a trustline balance is never read as a numeric amount yet). Both fail closed, a claim citing either is refused, not silently under-priced.

## What's real vs. what's a testnet stand-in

The protocol logic: scanning, ownership checks, the wrongful-liquidation price comparison, oracle signing, on-chain verification, is real code running against real testnet transactions throughout. Two assets are testnet stand-ins, disclosed here rather than left implicit: a mint standing in for USDC on Solana devnet (no canonical target exists there), and a minimal lending market on Sepolia we deploy ourselves, needed because staging a wrongful liquidation requires controlling the price a real market like Aave's testnet deployment would not let us control. Neither substitution touches the mechanism being demonstrated.
