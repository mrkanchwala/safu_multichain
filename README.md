# SAFU Staking Multichain

One USDC pool on Stellar that pays you back when a wallet you registered is drained, or when a loan in one of those wallets is wrongfully liquidated. Covered wallets can sit on Ethereum, Solana or Stellar. Payouts follow rules fixed in advance, and nobody votes on a claim.

Live on testnet at [safustaking.com](https://safustaking.com).

## How it works

1. **Stake USDC** from a Stellar, Ethereum or Solana wallet. Ethereum and Solana stakes travel to the Stellar pool over Circle CCTP v2. The stake lives on Stellar, and you never need a Stellar key. Payouts and withdrawals always return to the wallet you staked from.
2. **Register up to three covered wallets** on any mix of the three chains. You show each one is yours by sending a tiny amount from it to itself. A covered wallet is never the staking wallet, and it never connects to the site.
3. **File a claim** within 30 days of a drain. One claim can cite up to 20 drain transactions. The scanner checks each transaction on the chain it happened on, the oracle signs the result, and the pool contract checks that signature against its own limits before any money moves.
4. **Collect the payout.** A claim filed before your stake is 90 days old is recorded at full value and held until day 90. You then approve it, a 7-day cooldown runs, and the payout streams to you over 45 days.

**How much.** Each covered wallet has a tier (A, B or C) based on its on-chain history. The contract caps a payout at 15x your stake for Tier A, 10x for Tier B and 5x for Tier C, and the amount actually paid is the real loss up to that ceiling.

**Covered assets.** ETH, USDC and USDT on Ethereum; SOL, USDC and USDT on Solana; XLM, USDC and USDT on Stellar.

**Yield.** Idle USDC goes into a DeFindex vault on Blend. By default the yield on your own stake goes to you. Backers can add USDC that helps the pool pay large claims on time, without registering wallets of their own.

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
backend/                   Claim API (FastAPI): registry, ownership checks, loss pricing, oracle signing, fee relayer
src/                       The site (React + Vite), wallets on all three chains
config/                    One file per network: pool.testnet.json, pool.mainnet.json
scripts/deploy/            Contract deploy (deploy_v1.sh) and site deploy (deploy_site.sh)
scripts/e2e/               Live testnet end-to-end scripts
docs/                      Design, CCTP flow, testing, auditor disclosures
```

The three LayerZero folders hold an earlier registry design, kept for reference now that the registry is written directly on Stellar and cross-chain money moves over CCTP.

The backend imports the scanner and the tier engine from a separate private package and does not start without it, which is why backend tests stay out of CI.

## Networks

One setting picks the network for the whole pool: `SAFU_POOL_NETWORK=testnet` or `mainnet`. The backend, the site build and the nginx security headers all read `config/pool.<network>.json`. There is no default, and a blank value stops the backend. The deploy script writes contract addresses into that file. See [config/README.md](config/README.md).

## Tests

| Suite | Tests | Where it runs |
|---|---|---|
| Soroban contracts | 442 | VPS (CI runs the pool suite on `main`) |
| Contract fuzz targets | 6 | VPS, in a capped Docker container |
| Backend (pytest) | 632 | VPS |
| EVM (Foundry) | 21 | Local |
| Site (vitest) | 17 | Local and CI |
| Live testnet end to end | 6 staged hacks across 3 chains | Stellar testnet, Sepolia, Solana devnet |

What each suite covers, file by file, and how to run it: [docs/TESTING.md](docs/TESTING.md).

## Build and deploy the contracts

```bash
cd contracts
stellar contract build --package protection-pool   # same for covered-registry, cctp-adapter, safu-account
```

`scripts/deploy/deploy_v1.sh <config.env>` deploys all four contracts, reads every role back from the chain, and writes the new addresses into `config/pool.<network>.json`. A mainnet run also needs `CONFIRM_MAINNET=yes`. The config keys are listed in `scripts/deploy/testnet.env.example`.

## Status

Live on Stellar testnet, with Sepolia and Solana devnet for cross-chain staking. The Stellar contract has not had an external audit yet. Known, deliberate design choices are written up for the auditor in [docs/AUDITOR_DISCLOSURES.md](docs/AUDITOR_DISCLOSURES.md).

## License

Licensed under Apache 2.0, with the full text in [LICENSE](LICENSE).
