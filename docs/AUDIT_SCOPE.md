# Audit scope: SAFU protection pool (Stellar)

Prepared 2026-10-20 for the audit quote. This replaces the earlier scope built on
`mrkanchwala/safu-soroban` (2,443 lines, the Tranche 3 pool): that pool is wound down and is not
the audit target. The target is the new pool in this repository.

**Commit:** tag `audit-freeze-2026-10-20` (`85b2be5`) on `main`. This commit is deployed on Stellar
testnet; addresses and WASM hashes are in [TESTNET.md](TESTNET.md).

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

## 2. Out of scope

- `contracts-evm/src/MockLendingMarket.sol` (test fixture for liquidation tests)
- The LayerZero registry senders (`contracts-evm/src/RegistrySender.sol`, `contracts-solana/`,
  `contracts-layerzero/`): an earlier registry design kept for reference. The registry is now written
  directly on Stellar (`covered-registry`) and money moves over CCTP.
- The website (`src/`), deploy scripts (`scripts/`), all tests and fuzz targets
- The off-chain scanner and oracle signer (Python, separate repository). The contract trusts one
  oracle signature per claim and checks it against its own limits; see `docs/ARCHITECTURE.md`
  "Trust boundary".
- Third-party contracts the pool calls: DeFindex vault, Blend, Circle CCTP.

## 3. Read first

| Document | What it covers |
|---|---|
| `README.md` | What the pool does, in user terms |
| `docs/ARCHITECTURE.md` | System, claim sequence, trust boundary |
| `docs/DESIGN.md` | Rules and the reasons behind them |
| `docs/CCTP.md` | The cross-chain stake path |
| `docs/AUDITOR_DISCLOSURES.md` | Known-by-design behaviour, including the governance trust limit (2 of 3 roles = full control after 7 days) |
| `docs/TESTING.md` | How to build and run every test |
| STRIDE threat model | `docs/STRIDE_THREAT_MODEL.pdf` (2026-10-20, this pool) |

## 4. Whitelabel deployments and other yield venues

SAFU is sold to wallets, apps and protocols that run their own pool under their own name. We would
like the audit to cover that use, so a partner deployment does not need a new audit each time.

The lending protocol's own contracts are out of scope: SAFU only uses audited venues and relies on
their audits. The audit covers SAFU's code and its settings.

**Partner pools with their own settings.** A partner pool is a new deployment of the same audited
WASM (same hash), with its own roles and its own values for the adjustable settings. Every setting
has hard bounds and ordering rules in the code (`src/settings.rs`) and changes only through the coded
timelock. No code changes.
*Question for the auditor:* can a partner deployment of the audited WASM, with any settings inside
the coded bounds, rely on this audit without a new one? If not, would a short deployment review (WASM
hash, settings, roles and addresses checked against the audited tag) be enough, and what would it
cost per pool?

**A different lending protocol.** The pool deposits idle USDC into a DeFindex vault through three
calls (`deposit`, `withdraw`, `balance`, `src/vault.rs`), and governance can point it at another
vault (`GovChange::Vault`, two of three roles and 7 days). A partner who wants their own lending market
has two routes:
- a DeFindex vault whose strategy lends into their market: no pool code change (the strategy is
  DeFindex code, outside this scope; the DeFindex strategies we know of today lend on Blend);
- a direct connector in the pool for their market: new pool code.

*Questions for the auditor:*
- Does pointing the pool at another DeFindex vault, with the same three calls, stay inside this audit?
- What would a delta audit of one new direct lending connector cost, on top of this audit?

## 5. Testing done so far

- Unit and integration tests across all five contracts (`docs/TESTING.md`).
- Fuzzing: six targets across the pool, registry, account and CCTP adapter, no failures (`docs/TESTING.md` section 2).
- Mutation testing (cargo-mutants, CI): gaps closed with targeted tests; every mutant no test kills is
  listed with its reason in `docs/MUTATION_EQUIVALENTS.md`.
- Scout (CoinFabrik): run on every crate. The pool's "critical" results are all integer-overflow flags
  (the build aborts on overflow); every medium is reviewed in `docs/SCOUT_TRIAGE.md`.
