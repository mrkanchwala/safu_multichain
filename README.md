# SAFU Staking Multichain

One USDC pool on Stellar that pays you back when a wallet you registered is drained, or when a loan in one of those wallets is wrongfully liquidated. Covered wallets can sit on Ethereum, Solana or Stellar. Payouts follow rules set in advance, and nobody votes on a claim.

This is the new version of the pool: adjustable settings, upgrades behind two-of-three governance, a backer withdrawal drip, a first-come claim line and upkeep inside every user call. It goes to testnet first, then to audit. The pool live today at [safustaking.com](https://safustaking.com) is the earlier version (tag `mainnet-build-2026-09-29-r3`), and its stakers move to the new pool after the audit.

## How it works

1. **Stake USDC** from a Stellar, Ethereum or Solana wallet. Ethereum and Solana stakes travel to the Stellar pool over Circle CCTP v2. The stake lives on Stellar, and you never need a Stellar key. Payouts and withdrawals always return to the wallet you staked from. Anything above the maximum stake goes into backing: it earns yield and is never forfeited on a claim. You can withdraw any time, unless a claim on your stake is open.
2. **Register up to three covered wallets** on any mix of the three chains. You show each one is yours by sending a tiny amount from it to itself. A covered wallet is never the staking wallet, and it never connects to the site.
3. **File a claim** within 30 days of a drain (the window is a setting). One claim can cite up to 20 drain transactions. The scanner checks each transaction on the chain it happened on, the oracle signs the result, and the pool contract checks that signature against its own limits before any money moves.
4. **Collect the payout.** A claim filed before your stake is 90 days old is recorded at full value and held until day 90. You then approve it in one call, a 7-day cooldown runs, and the payout streams to you over 45 days. If the pool is short of room on a given day, claims wait in a line and are released strictly in the order they joined. A paid claim forfeits the stake, and that address cannot stake again; a fresh address with new covered wallets can.

**How much.** Each covered wallet has a tier (A, B or C) based on its on-chain history. The contract caps a payout at 15x your stake for Tier A, 10x for Tier B and 5x for Tier C (adjustable within 1x to 20x; a cut applies only to new stakes, a raise to everyone), and the amount actually paid is the real loss up to that ceiling.

**Covered assets.** ETH, USDC and USDT on Ethereum; SOL, USDC and USDT on Solana; XLM, USDC and USDT on Stellar.

**Yield.** Idle USDC goes into a DeFindex vault on Blend. By default the yield on your own stake goes to you, and a claim never takes the yield you already earned. Backers can add USDC that helps the pool pay large claims on time, without registering wallets of their own; backing earns yield from day one and leaves through a daily drip.

**Points.** One point per dollar per day, for stakers and backers at separately set rates. Points stay with you when you withdraw and are burned on a paid claim.

**Changing the rules.** Each adjustable number has hard bounds and changes only through a proposal, a second role's approval and 7 days in public. A code upgrade takes the same two roles and 7 days. Values a stake or claim depends on are fixed onto it, so a change never reaches someone already in. Details: [docs/AUDITOR_DISCLOSURES.md](docs/AUDITOR_DISCLOSURES.md).

## Repository layout

```
contracts/                 Soroban contracts (Rust, soroban-sdk 27, wasm32v1-none)
  protection-pool/           The pool: stake, back, claims, payout streams, settings, governance, vault
  covered-registry/          Which wallets each stake covers (up to 3, bound for good)
  cctp-adapter/              Stake or back from Ethereum and Solana over Circle CCTP v2
  safu-account/              Per-user Stellar account controlled by an Ethereum or Solana key
  cctp-common/               Shared CCTP message parsing
contracts-evm/             Foundry: a mock lending market for liquidation tests, a LayerZero registry sender
contracts-solana/          Solana LayerZero registry sender
contracts-layerzero/       Vendored LayerZero code
src/                       The site (React + Vite), wallets on all three chains
config/                    One file per network: pool.testnet.json, pool.mainnet.json
scripts/deploy/            Contract deploy (deploy_v1.sh) and vault setup (setup_vault.sh)
scripts/e2e/               Fast-build guard: the testnet build differs from this one only in time constants
docs/                      Design, architecture, CCTP flow, testing, auditor disclosures, audit scope
```

The three LayerZero folders hold an earlier registry design, kept for reference now that the registry is written directly on Stellar and cross-chain money moves over CCTP.

The claim backend (scanner, tier engine, loss pricing, oracle signing, fee relayer), the server configuration and the live testnet scripts are kept in a private repository. Scoring rules are not published, so a claim cannot be shaped to pass them.

## Networks

One setting picks the network for the whole pool: `SAFU_POOL_NETWORK=testnet` or `mainnet`. The site build here and the private backend both read `config/pool.<network>.json`. There is no default, and a blank value stops the backend. The deploy script writes contract addresses into that file. See [config/README.md](config/README.md).

## Tests

| Suite | Tests | Where it runs |
|---|---|---|
| Soroban contracts | 549 | Build server (CI runs the pool suite) |
| Contract fuzz targets | 6 | Build server, in a capped Docker container |
| Mutation testing (cargo-mutants) | sharded runs | GitHub Actions |
| Static analysis (Scout) | all 5 crates | GitHub Actions |
| Backend (pytest, private repository) | 632 (2026-09-29) | Build server |
| EVM (Foundry) | 21 | Local |
| Site (vitest) | 19 | Local and CI |
| Live testnet end to end (private repository) | 6 staged hacks across 3 chains | Stellar testnet, Sepolia, Solana devnet |

What each suite covers, file by file, and how to run it: [docs/TESTING.md](docs/TESTING.md). What is in and out of audit scope: [docs/AUDIT_SCOPE.md](docs/AUDIT_SCOPE.md).

## Build and deploy the contracts

```bash
cd contracts
stellar contract build --package protection-pool   # same for covered-registry, cctp-adapter, safu-account
```

`scripts/deploy/deploy_v1.sh <config.env>` deploys all four contracts, reads every role back from the chain, and writes the new addresses into `config/pool.<network>.json`. A mainnet run also needs `CONFIRM_MAINNET=yes`. The config keys are listed in `scripts/deploy/testnet.env.example`.

`scripts/deploy/setup_vault.sh` creates the DeFindex vault: a separate seeder wallet makes the first deposit (so the shares DeFindex locks at the first deposit never belong to the pool), everything is moved into Blend, and the script refuses to go on unless the vault's idle balance is 0. It then funds the pool's rounding reserve. `DRY_RUN=1` simulates only.

## Status

The earlier version of this pool is live on Stellar mainnet. This version is headed to testnet, then to audit through the Stellar Community Fund's Audit Bank. Neither has had an external audit yet. Known, deliberate design choices are written up for the auditor in [docs/AUDITOR_DISCLOSURES.md](docs/AUDITOR_DISCLOSURES.md).

## License

Licensed under Apache 2.0, with the full text in [LICENSE](LICENSE).
