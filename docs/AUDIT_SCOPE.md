# Audit scope: SAFU protection pool (Stellar, with the Solana port)

Prepared 2026-10-20 for the audit quote. This replaces the earlier scope built on
`mrkanchwala/safu-soroban` (2,443 lines, the Tranche 3 pool): that pool is wound down and is not
the audit target. The target is the new pool in this repository.

**Branch:** `main`. The exact commit is pinned with a tag once the pool is on testnet; until then,
review the head of `main`. Testnet deployment is due this week.

## 1. In scope: Stellar (Soroban, Rust, soroban-sdk 27.0.0, `wasm32v1-none`)

| Contract | Path | Lines (source, no tests/comments/blank) | What it does |
|---|---|---|---|
| protection-pool | `contracts/protection-pool/src/` | 4,589 | Stake, back, claims and the claim line, payout streams, settings, governance and upgrades, DeFindex/Blend vault, yield and points |
| safu-account | `contracts/safu-account/src/` | 405 | Per-user Stellar account controlled by an Ethereum or Solana key (custom `__check_auth`) |
| cctp-common | `contracts/cctp-common/src/` | 296 | Shared Circle CCTP v2 message parsing |
| cctp-adapter | `contracts/cctp-adapter/src/` | 187 | Stake or back from Ethereum and Solana over CCTP v2 |
| covered-registry | `contracts/covered-registry/src/` | 135 | Which wallets each stake covers (up to 3, bound for good) |
| **Total** | | **5,612** | |

Line counts exclude `src/test/`, `fuzz/` and comment/blank lines. The pool WASM is 130,732 bytes against
the network maximum of 131,072.

## 2. In scope: Solana (Anchor 1.1.2), repository `mrkanchwala/safu_solana`

| Part | Path | Lines | What it does |
|---|---|---|---|
| safu_pool program | `programs/safu_pool/src/` | 3,287 | Stake, claims, backers, upkeep, admin, Marinade staking |
| pool-core | `crates/pool-core/src/` | 700 | Every pool rule as a pure function |
| **Total** | | **3,987** | |

The Solana pool is being brought to the same rules as the Stellar pool. Its commit is pinned before the
audit starts; the line count above is the current tree and is the one to quote against for now.

## 3. Out of scope

- `contracts-evm/src/MockLendingMarket.sol` (test fixture for liquidation tests)
- The LayerZero registry senders (`contracts-evm/src/RegistrySender.sol`, `contracts-solana/`,
  `contracts-layerzero/`): an earlier registry design kept for reference. The registry is now written
  directly on Stellar (`covered-registry`) and money moves over CCTP.
- The website (`src/`), deploy scripts (`scripts/`), all tests and fuzz targets
- The off-chain scanner and oracle signer (Python, separate repository). The contract trusts one
  oracle signature per claim and checks it against its own limits; see `docs/ARCHITECTURE.md`
  "Trust boundary".
- Third-party contracts the pool calls: DeFindex vault, Blend, Circle CCTP, Marinade.

## 4. Read first

| Document | What it covers |
|---|---|
| `README.md` | What the pool does, in user terms |
| `docs/ARCHITECTURE.md` | System, claim sequence, trust boundary |
| `docs/DESIGN.md` | Rules and the reasons behind them |
| `docs/CCTP.md` | The cross-chain stake path |
| `docs/AUDITOR_DISCLOSURES.md` | Known-by-design behaviour, including the governance trust limit (2 of 3 roles = full control after 7 days) |
| `docs/TESTING.md` | How to build and run every test |
| STRIDE threat model | Being redone for this pool; sent separately |

## 5. Testing done so far (Stellar pool)

- Unit and integration tests across all five contracts (`docs/TESTING.md`).
- Fuzzing: solvency, backers and override targets, 142,000 runs, no failures.
- Mutation testing (cargo-mutants, CI): gaps closed with targeted tests; every mutant no test kills is
  listed with its reason in `docs/MUTATION_EQUIVALENTS.md`.
- Scout (CoinFabrik): run on every crate. The pool's "critical" results are all integer-overflow flags
  (the build aborts on overflow); every medium is reviewed in `docs/SCOUT_TRIAGE.md`.
