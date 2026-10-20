# Testing

Every suite in this repository: what it proves, how to run it, and where it runs. Contract counts are from 2026-10-20 on `main`, the pool going to audit.

| Suite | Tests | Where it runs |
|---|---|---|
| Soroban contracts | 553 | Build server (CI runs the pool suite) |
| Contract fuzz targets | 6 | Build server, in a capped Docker container |
| Mutation testing (cargo-mutants) | sharded runs | GitHub Actions, started by tag |
| Static analysis (Scout) | all 5 crates | GitHub Actions, started by tag |
| Backend (pytest, private repository) | 632 (2026-09-29) | Build server |
| EVM (Foundry) | 21 | Local |
| Site (vitest) | 19 | Local and CI |
| Live testnet end to end (private repository) | 6 staged hacks across 3 chains | Stellar testnet, Sepolia, Solana devnet |

## 1. Soroban contract tests (553)

```bash
cd contracts
cargo test --workspace            # all five crates
cargo test -p protection-pool     # the pool only (what CI runs)
```

The pool suite takes about 4 minutes, most of it in `pool_solvency_tests.rs`. Run it on a build server; it holds 2 to 3 CPU cores for the whole run.

### protection-pool (486)

| File | Tests | What it checks |
|---|---|---|
| `claim_tests.rs` | 70 | Claim filing, the 90-day queue, the 100-day approval window, suspension, stale-claim expiry, payout streaming |
| `backer_tests.rs` | 38 | Backing, the optional maturity clock per deposit, the daily withdrawal drip and each day's fair share, money needed by open claims cannot leave, backer yield |
| `pool_rules_tests.rs` | 34 | The claim line (release in order, a fair share of each day's payout cap), no expiry clock while the pool can't pay, the pause gap, suspensions that lapse, upgrades (two roles, 7 days, storage version and migrate), stake overflow into backing, the rounding reserve |
| `d1_signature_tests.rs` | 30 | The oracle's Ed25519 signature: every tampered field, wrong contract or key, deadlines, key rotation |
| `t3_flags_tests.rs` | 29 | Claims queued while capacity is short, and yield accounting |
| `inpath_liquidity_tests.rs` | 28 | Moving money between pool and vault inside ordinary user calls, including vault failures and theft attempts |
| `yield_tests.rs` | 27 | Staker and backer yield, taken any time; the pending-yield view; points as dollar-days |
| `stake_tests.rs` | 27 | Stake bounds, the pool cap, beneficiary rules, withdrawal, emergency exit |
| `settings_tests.rs` | 28 | Adjustable settings: allowed ranges, the order between related settings, the 7-day timelock, snapshots on stakes and claims |
| `mutation_gap_tests.rs` | 27 | Cases added where mutation testing showed a changed line went unnoticed |
| `admin_tests.rs` | 25 | Setup, role changes through governance, the pool cap, pause, stake suspension |
| `d2_vault_tests.rs` | 25 | Depositing idle funds into the yield vault and withdrawing them |
| `override_tests.rs` | 22 | The two-key override: both keys must agree, and an override can never exceed a tier ceiling |
| `solvency_tests.rs` | 16 | Tier ceilings, the pool never commits more than it holds, points, eligibility |
| `governance_tests.rs` | 16 | Three-role governance: two of three to pass a change, 7-day delay, recovery after a lost key |
| `t2_mutation_gap_tests.rs` | 12 | More mutation-testing gap cases |
| `line_and_watch_tests.rs` | 7 | The claim line and the watch list, driven one step at a time over many random layouts against a plain model |
| `p2_findings_tests.rs` | 6 | Regression tests for two findings from an internal security review |
| `edge_tests.rs` | 5 | Single rules pinned to their exact boundary |
| `blend_scenario_tests.rs` | 5 | What the pool does if the Blend lending market behind the vault is exploited |
| `profiling_tests.rs` | 5 | CPU and memory cost of the heaviest calls, against Soroban's limits |
| `pool_solvency_tests.rs` | 3 | Solvency under the pool's liquidity model |
| `pool_demo_tests.rs` | 1 | One full walkthrough: stake, claim, stream, withdraw |

`common.rs` holds shared setup and has no tests of its own.

### Other contracts (67)

| Crate | Tests | What it checks |
|---|---|---|
| `cctp-adapter` | 30 | Full CCTP flows against Circle's real CCTP v2 contracts: stake (with any amount above the maximum going to backing), back, withdraw home, payouts home |
| `safu-account` | 16 | The account accepts only its owner's own Ethereum or Solana signature |
| `covered-registry` | 13 | One test per registry rule: a wallet binds to one stake for good, at most 3, only the writer key can add |
| `cctp-common` | 8 | CCTP message parsing |

## 2. Fuzz targets (6)

A fuzz target throws long random sequences of actions at a contract and checks after every action that the money rules still hold. Normal tests cover the cases someone thought of; fuzzing looks for the ones nobody did.

| Target | Crate | What must always hold |
|---|---|---|
| `fuzz_solvency` | protection-pool | The pool never owes more than it holds, across random stakes, withdrawals, claims, streams, cancels and time jumps |
| `fuzz_backers` | protection-pool | Every backer's money is accounted for exactly: waiting deposits, drip-paid withdrawals, stake overflow into backing, the full claim state machine |
| `fuzz_override` | protection-pool | The two-key override and key rotation never double-pay or leave a stale approval |
| `fuzz_registry` | covered-registry | The registry always matches a shadow model of its rules, re-read after every action |
| `fuzz_check_auth` | safu-account | Only the owner's real signature moves money: mutated, foreign or malformed signatures always fail |
| `fuzz_cctp_flows` | cctp-adapter | CCTP messages cannot be tampered with, replayed or redirected, with Circle's real contracts in the loop |

Last runs, all clean:

| Target | Runs | Date | Code |
|---|---|---|---|
| `fuzz_solvency` | 8,269 | 2026-10-20 | `6e61cf4` |
| `fuzz_backers` | 6,697 | 2026-10-20 | `6e61cf4` |
| `fuzz_override` | 9,325 | 2026-10-20 | `6e61cf4` |
| `fuzz_registry` | 9,507 | 2026-10-20 | `6e61cf4` |
| `fuzz_check_auth` | 42,340 | 2026-10-20 | `6e61cf4` |
| `fuzz_cctp_flows` | 674 | 2026-10-20 | `6e61cf4`; each run deploys Circle's contracts and the pool, so runs are slow (23,189 runs on 2026-09-24, earlier build) |

```bash
cd contracts/protection-pool
cargo fuzz run fuzz_solvency -- -max_total_time=300
```

Run fuzzing on a Linux server inside a Docker container with fixed CPU and memory limits. On macOS the address sanitizer crashes before any input runs.

## 3. Mutation testing

`cargo-mutants` changes one operator or line at a time in the pool and checks that some test fails. A change no test notices is a gap. `.github/workflows/mutants.yml` runs it on GitHub Actions, split into parallel shards, started by pushing a tag:

| Tag | What runs |
|---|---|
| `mutants-smoke-<n>` | One small shard, to prove the setup gives real results |
| `mutants-full-<n>` | Every mutant in pool code changed since `main`, 20 shards |
| `mutants-check-<n>` | Only the functions listed in `.github/mutants-check.args` |

Each shard uploads its caught, missed, timeout and unviable lists as an artifact.

| Run | Code | Caught | Missed | Notes |
|---|---|---|---|---|
| `mutants-full-1` | `d08eef3` | 876 | 115 | 7 timeouts, 13 unviable |
| `mutants-check-2` | `999b722` | 375 | 54 | Re-check of every function with a miss, after new tests |
| `mutants-check-3` | `b1c74ce` | 115 | 7 | Re-check of the functions targeted by the 2026-10-20 tests. The 7 left are explained in `docs/MUTATION_EQUIVALENTS.md` |
| `mutants-check-4` | `76a7796` | 0 | 0 | The filter matched no mutants (only whole-function replacements exist for `apply`). Covered by hand instead: removing the new writer signature check, or emptying `apply`, fails the governance tests |
| `mutants-check-5` | `6e61cf4` | 177 | 11 | Functions changed by the 2026-10-20 fixes; 2 timeouts. 7 misses closed with new tests (each confirmed to fail on its mutant); 4 equivalent, explained in `docs/MUTATION_EQUIVALENTS.md` |
| `mutants-check-7` | `e3fccb2` | 53 | 0 | Re-check of `submit_claim` and `emergency_exit` after the new tests: every mutant caught |

A missed mutant is closed in one of two ways: a test that fails on it, or a written reason it cannot change behaviour (equivalent or unreachable). Those reasons are shared with the auditor.

## 4. Static analysis (Scout)

`.github/workflows/scout.yml` runs CoinFabrik's Scout over all five crates, started by a `scout-<n>` tag. Last run `scout-4` (2026-10-20), on the contract code at `6e61cf4`, which is the code under tag `audit-freeze-2026-10-20`:

| Crate | Critical | Medium | Enhancement |
|---|---|---|---|
| protection-pool | 196 | 22 | 49 |
| covered-registry | 0 | 7 | 3 |
| safu-account | 4 | 4 | 2 |
| cctp-adapter | 1 | 3 | 2 |
| cctp-common | 5 | 0 | 1 |

Every "critical" result is the integer-overflow lint. The release profile sets `overflow-checks = true`, so an overflow aborts the call and changes nothing. Every medium result is reviewed in [`SCOUT_TRIAGE.md`](SCOUT_TRIAGE.md); none is open.

## 5. Backend (632, as of 2026-09-29)

The backend lives in a private repository with the scanner and the tier engine, so its tests are listed here without the code. The suite covers:

- Claim filing end to end: one or many transactions, the 20-drain limit, the 30-day window
- Covered-wallet registry: ownership proof, the on-chain registry writer, races between two registrations
- Loss pricing: the price at the moment of loss, per asset and chain
- Wrongful-liquidation claims: reading the liquidation, checking the lender, pricing the loss
- Oracle signing: payload layout matching the contract byte for byte
- The fee relayer for Ethereum and Solana users
- The network switch: a mainnet pool refuses test settings and a testnet scanner
- Error messages: nothing internal leaks to the user

## 6. EVM (21)

```bash
cd contracts-evm
forge test
```

| File | Tests | What it checks |
|---|---|---|
| `MockLendingMarket.t.sol` | 15 | The mock lending market used to stage liquidations on Sepolia |
| `RegistrySender.t.sol` | 6 | The LayerZero registry sender (earlier design, kept for reference) |

## 7. Site (19)

```bash
npm install
npm run test     # vitest
npm run build    # type check and production build
npm run lint     # oxlint
```

`src/lib/friendly-error.test.ts` checks that contract and wallet errors turn into plain-English messages.

## 8. Live testnet end to end

These scripts ran the real contracts (the earlier pool build, before this rework), the real backend and real Circle CCTP against Stellar testnet, Sepolia and Solana devnet. They sit in the private repository with the backend they drive. The fast-build guard below is public.

### Fast build

Real waiting periods (a 90-day queue, a 7-day cooldown, 45 days of streaming) cannot be tested live in an afternoon. `scripts/e2e/build_fast.sh` copies the contracts, shortens only the time constants listed in `scripts/e2e/fast_constants.txt` to minutes, and builds both trees. It fails unless the two trees differ in exactly those lines and nothing else, and unless the other three contracts come out byte-identical. The testnet runs used that fast build; the deployable build is the unmodified `contracts/` tree.

### Scripts (private repository)

| Script | What it does |
|---|---|
| `setup.py` | Creates throwaway test identities, USDC trustlines and funding |
| `runner.py` | Runs the contract path matrix, one scenario per group |
| `groups_b.py` | Stake from an Ethereum wallet over CCTP, then bring the money home |
| `groups_bb.py` | Back the pool from an Ethereum or Solana wallet over CCTP |
| `groups_de.py` | Covered-wallet registry and claims, signed by the production registry and oracle keys |
| `groups_i.py` | The yield vault and in-path rebalancing, on a pool with a real DeFindex vault |
| `hacks.py`, `hack_steps.py` | Six real drains across the three chains, each claimed through the real backend |
| `payouts.py` | Back the pool, then collect approved claims on all three chains |
| `relay_proof.py` | The fee relayer over HTTP, through the real app |
| `token_cases_solana.py`, `token_cases_stellar.py` | Labeled USDC and USDT drain and clean cases for the scanner |
| `cctp_bridge.py`, `solana_cctp.py` | Move test USDC across chains with CCTP, no faucet needed |
| `home_finish_check.ts` | Runs the site's own cross-chain finish step in Node against a live transfer |
| `vps_sign.py` | Signs test payloads with a production key on the server, so keys never leave it |
| `preview_backend.py` | A local backend for previewing the site against testnet |

### Real-values smoke

`smoke_realvalues.py` checks a deploy of the unmodified build with the production role keys, covering a stake, a registry write and a claim signed by the KMS keys, the claim held in the 90-day queue, and every early action refused by the real clocks (releasing the claim, raising the pool cap, a settings change and a governance change, where the co-signer votes with its KMS key). The last run, on 2026-09-29, passed 17 of 17 steps.

Deploy records for each run are written to `deploy/out/` (not committed).
