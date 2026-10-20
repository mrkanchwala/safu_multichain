# SAFU Protection Pool on Stellar: design

## The problem

Two things happen to crypto wallets and positions that no existing product covers together: a wallet gets drained by phishing, an approval exploit, or a stolen key; or a lending position gets liquidated on a price that was never real. Both are losses a user did nothing wrong to suffer, and both are usually met with either nothing, or a discretionary compensation decision taken by a committee weeks later.

SAFU pays for both, automatically, from one pool. Stake USDC once; it covers a wallet drain on Ethereum, Solana or Stellar, or a wrongful liquidation on a listed lending market, at a deterministic multiple of your own stake. No vote, no appeal, no human decides.

## One pool, not two contracts

This build extends SAFU's own protection-pool, the same contract design already running on Stellar mainnet. No new lending market was built. That choice follows directly from a simple fact: `submit_claim` doesn't care *why* a loss happened. It checks an oracle signature, a tier ceiling, and a real-loss cap. Wrongful liquidation slots into that same claim path as a second detected cause alongside wallet drain: the only new part is an off-chain detector, and the contract stays the same.

The pool is USDC-denominated. Whatever asset was lost, ETH, USDC, USDT on Ethereum, SOL on Solana, XLM on Stellar, the payout always settles in USDC from this one pool on Stellar. Stellar holds the money and the price truth for every covered chain (see "Pricing" below).

## Coverage, by tier

A wallet's on-chain history sets its tier, computed off-chain from age and activity: older, quieter wallets score higher. Tier sets a coverage *ceiling*: the payout can be any amount up to it. The defaults are below; each ceiling is a setting between 1x and 20x, with A ≥ B ≥ C. Each stake keeps the ceilings in force on its stake day, and a claim uses the higher of that and the current value, so a cut reaches only new stakes and a raise reaches everyone:

| Tier | Ceiling |
|---|---|
| A | 15x stake |
| B | 10x stake |
| C | 5x stake |

The actual entitlement is `min(ceiling, real loss)`, capped by the ceiling but never invented above it. Yield later credited to a stake never raises this ceiling, it's anchored to the fixed amount admitted at stake time, which the contract already bounds to a small fraction of the pool cap. Staking is permissionless: anyone can stake inside the bounds, no whitelist. One stake per address. A deposit above the maximum stake is split: the maximum is staked and the rest becomes backing, which earns yield and is never forfeited. A pool full for stakes refuses the whole deposit.

## Ownership: who can claim what

A staker registers up to three wallets they want covered, on any mix of chains, **before** anything happens to them. A claim can only name a wallet already on that list, checked against a timestamp that must predate the loss.

This closes a real gap that a naive multi-chain design opens: without it, a stranger's public drain transaction could be cited by anyone as their own loss. Pre-registration means the binding exists before the loss does, so citing someone else's drain afterward gains nothing. It does not require a signature from the drained wallet itself, deliberately, since key compromise is a covered vector, and demanding a signature from a compromised wallet would authenticate the thief.

## Pricing

Every covered asset is priced through Reflector, the Stellar-native oracle network, read from its mainnet feed regardless of which chain the loss happened on. The price used for a payout is captured at the moment the loss is detected, not looked up later when the claim is filed, since a filing can happen up to 30 days after the fact and Reflector's own retention window is far shorter than that. Capturing early and storing the result makes the entitlement both correct at the time and auditable afterward.

## Wrongful liquidation

A lending position liquidated on a manufactured price is a covered loss, on any of the three chains. The detector compares the price a liquidation was executed at against Reflector's own price history over a window ending at that liquidation, never against the lending market's own feed, which is exactly the tick under question. A liquidation priced inside that comparison is a genuine market move and pays nothing; one priced meaningfully outside it is treated as fed a manufactured price, and the claim path opens.

Self-liquidation needs no special handling: the check evaluates the price, never who triggered the liquidation. If a lending market really was fed a bad price, the loss is real regardless of who benefits from reporting it, and paying it is the correct outcome.

The exact comparison window and threshold are not published here, for the same reason SAFU's fraud-scoring internals never are: publishing the exact numbers makes the check gameable. The structural mechanism, an independent price history, compared over a window, with a flag on excess deviation, is the part that matters and the part that's disclosed.

## On-chain and off-chain

| On-chain (changed only through two roles and 7 days in public) | Off-chain (Python, changeable) |
|---|---|
| Tier ceilings (15x / 10x / 5x) | Which wallets are registered to which staker |
| `entitlement > ceiling` rejection | The wrongful-liquidation price check |
| Oracle signature verification | Reflector price reads |
| 90-day time gate (can only go up), cooldown, vesting | The actual entitlement number |
| Solvency and daily outflow caps | Real-loss capping |

The contract never learns what was lost, on which chain, or why the claim was ruled wrongful: it verifies a signed number against a ceiling. Everything that decides *that* number is off-chain and can be corrected without touching the deployed contract.

## Yield

Staked USDC deploys into a DeFindex vault targeting Blend's USDC pool. Yield is simple yield on principal, tracked through a growing index rather than per-user share tokens, which avoids a known share-price manipulation pattern. Stakers and backers each get their share (all of it by default; separate settings for each side). Yield stays in the pool, deployed, until its owner takes it, and is paid in full whenever asked; until then it can serve as cash for claims but never counts as capacity. A claim never takes the claimant's earned yield. Growth on unclaimed yield and on SAFU's own balance is protocol revenue.

## Claims: a line, not a race

When the pool has no room for a claim on a given day (daily admission cap, capacity, or cash), the claim joins a line. Queued claims are released strictly in the order they joined, and a new claim joins the back while anyone waits, even if there is room for it. Each streaming claim gets its own pro-rata share of the day's payout cap, so collecting early in the day takes nothing from anyone else. No expiry clock runs on a day the pool could not pay.

## Backing

Backers add capital without covering wallets. Backing earns yield and counts toward the pool from day one (a maturity delay is available as a setting, default 0). Withdrawal is a daily drip: 7 days when the pool is idle, up to 90 days when it is busy, recomputed daily and shared pro rata between backers. Only capital not needed by open claims can leave.

## No keeper

Every user transaction does a small, capped amount of upkeep: release the head of the claim line if there is room, move due claims forward, clear one or two finished entries, and harvest yield. Nothing depends on a bot or a server job to keep the pool moving.

## Settings and upgrades

About two dozen numbers are adjustable inside hard bounds (stake limits, daily caps, tier ceilings, the claim window and time gate, cooldown, vesting, backer drip, points rates and more). Each change needs a proposal, a second role's approval of the same value, and 7 days in public. The contract can also be upgraded, through the same two roles and 7 days, never through the single-role recovery path. The trust limit is plain: two of the three roles control the pool after 7 days, and stakers can always leave in that window.

## What is deliberately not covered

Covered assets are ETH, USDC and USDT on Ethereum, SOL, USDC and USDT on Solana, and XLM, USDC and USDT0 on Stellar. A wrongful-liquidation claim pays only on a listed lending market and only when that market's own price feed failed; where the scanner cannot yet read a market's liquidation bonus, the claim is refused rather than guessed. Everything outside these lines fails closed: a claim citing it is refused outright.

## What's real vs. what's a testnet stand-in

The protocol logic: scanning, ownership checks, the wrongful-liquidation price comparison, oracle signing, on-chain verification, is real code running against real testnet transactions throughout. Two assets are testnet stand-ins, disclosed here rather than left implicit: a mint standing in for USDC on Solana devnet (no canonical target exists there), and a minimal lending market on Sepolia we deploy ourselves, needed because staging a wrongful liquidation requires controlling the price a real market like Aave's testnet deployment would not let us control. Neither substitution touches the mechanism being demonstrated.
