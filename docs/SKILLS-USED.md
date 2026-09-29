# Stellar resources used during development

The handbook asks teams to cite the specific skill files used while building, by path, and to note whether the `stellar-build` skill installer, Raven MCP, or skills.stellar.org was used.

This file records only what was genuinely consulted. It is kept accurate rather than complete, because a citation that cannot be substantiated is worse than a missing one.

## Tooling

| Resource | Used | Notes |
|---|---|---|
| `stellar` CLI | Yes | Version 28.0.0. Used for contract inspection and, at deploy time, for installing and invoking. |
| `soroban-sdk` | Yes | Version 27.0.0, targeting `wasm32v1-none`. |
| `@stellar/stellar-sdk` (Python) | Yes | Used off-chain for the oracle-signing infrastructure, payload construction and Ed25519 signing over the same byte format the contract verifies. |
| `@stellar/stellar-sdk` (JS) | Yes | Transaction building, simulation and submission from the dapp. |
| `@creit.tech/stellar-wallets-kit` | Yes | Version 2.6.0. Wallet connection. API verified against the published package source rather than written from memory. |
| stellar-build skill installer | No | |
| Raven MCP (raven.stellar.buzz) | No | |

## Skill files

Read directly from `stellar/stellar-dev-skill`, in an isolated Codespace created
on that repository. Each row names the specific decision the file informed.

| Skill file | What it informed in this build |
|---|---|
| `skills/smart-contracts/SKILL.md` | The storage model this build extends and does not merely inherit: two new instance-level singletons (a growing yield index, an explicit protocol-yield counter) added as typed `DataKey` variants, matching the existing pattern in the forked pool exactly. The new yield-split event uses the current `#[contractevent]` form, since the tuple-topic form is deprecated. |
| `skills/smart-contracts/SKILL.md` (security guidance) | Checked arithmetic on every new line this build added (the workspace builds with `overflow-checks = true`, unchanged); no new upgrade surface introduced; the rule that time-to-live is never a security boundary carried through unmodified, since none of this build's new state depends on TTL for anything but liveness. |
| `skills/assets/SKILL.md` | Confirmed the pool's XLM/USDC handling already treats a classic Stellar asset as 7 decimals, not assumed 6, checked directly against this contract rather than carried over from an unrelated one, since a wrong assumption here is exactly the bug class that already cost real debugging time once this build (a hardcoded 8-decimal literal on the frontend, corrected before it shipped). |

The full official set, for reference: `agentic-payments`, `assets`, `cross-chain`,
`dapp`, `data`, `smart-contracts`, `standards`, `zk-proofs`.

## What the security checklist changed

`skills/smart-contracts/security.md` has a section for token-consuming contracts, which covers a staking pool directly. Working through it against this build's own additions as well as the pool it forks found no new open items. The checklist's decimals-assumption warning applies to other Stellar contracts; this pool queries and stores decimals off-chain, per asset, in the loss-derivation and price-conversion modules, and assumes a fixed value nowhere.

One open item carries over from the base pool, unchanged by this build: the anchor's own documentation names a missing USDC trustline as a real stall condition, and the dapp surfaces that state once it happens rather than checking for it beforehand.

The rest of the contract checklist is met: authorization loaded from storage and re-checked at every layer, one-shot initialization, external contract addresses allowlisted and never caller-supplied, checked arithmetic with signs validated, typed storage keys, proactive TTL extension with no TTL-as-security assumption, bounded loops, events on auditable state changes, error codes appended rather than renumbered, and a pause control. There is no upgrade path at all, which is the strongest available answer to the upgrade row.

## Integration partners

| Partner | Status |
|---|---|
| Reflector | Live price feed, read from the mainnet oracle for every covered asset (ETH, SOL, USDC, USDT, XLM), regardless of which testnet a loss was detected on. MIT licensed, free at the Pulse tier, audited by Code4rena and OtterSec. |
| DeFindex → Blend | Staked USDC deploys through DeFindex into Blend's USDC pool for yield, the vault integration the base protection-pool already carries, unmodified by this build except for the split accounting on top of it. |

## Why Stellar

Reflector's price feed and the protection pool's settlement both live on Stellar, which is what makes the multichain claim credible rather than aspirational: a loss detected on Ethereum or Solana is priced by the same oracle, from the same chain, that the payout settles on. Stellar isn't one of three chains this build touches equally, it's the one piece everything else routes through.

The fiat leg also exists here already. Stellar's anchor network is a real on and off ramp rather than a planned one, which is what makes a local-currency entry point to USDC-denominated protection buildable rather than theoretical.
