# Testing

Every suite in this repository: what it proves, how to run it, and where it runs. Counts are from runs on 2026-09-29 at contracts commit `4970bc7`.

| Suite | Tests | Where it runs |
|---|---|---|
| Soroban contracts | 442 | VPS (CI runs the pool suite on `main`) |
| Contract fuzz targets | 6 | VPS, in a capped Docker container |
| Backend (pytest, private repository) | 632 | VPS |
| EVM (Foundry) | 21 | Local |
| Site (vitest) | 17 | Local and CI |
| Live testnet end to end (private repository) | 6 staged hacks across 3 chains | Stellar testnet, Sepolia, Solana devnet |

## 1. Soroban contract tests (442)

```bash
cd contracts
cargo test --workspace            # all five crates
cargo test -p protection-pool     # the pool only (what CI runs)
```

The pool suite takes about 6 minutes. Run it on a build server; it holds 2 to 3 CPU cores for the whole run.

### protection-pool (380)

| File | Tests | What it checks |
|---|---|---|
| `claim_tests.rs` | 58 | Claim filing, the 90-day queue, the 100-day approval window, suspension, stale-claim expiry, payout streaming |
| `backer_tests.rs` | 32 | Backing, the 7-day maturity, the withdrawal notice, money needed by open claims cannot leave |
| `d1_signature_tests.rs` | 30 | The oracle's Ed25519 signature: every tampered field, wrong contract or key, deadlines, key rotation |
| `t3_flags_tests.rs` | 29 | Claims queued while capacity is short, and yield accounting |
| `stake_tests.rs` | 27 | Stake bounds, the pool cap, beneficiary rules, withdrawal, emergency exit |
| `mutation_gap_tests.rs` | 26 | Cases added where mutation testing showed a changed line went unnoticed |
| `admin_tests.rs` | 25 | Setup, role changes through governance, the pool cap, pause, stake suspension |
| `d2_vault_tests.rs` | 25 | Depositing idle funds into the yield vault and withdrawing them |
| `inpath_liquidity_tests.rs` | 24 | Moving money between pool and vault inside ordinary user calls, including vault failures and theft attempts |
| `override_tests.rs` | 22 | The two-key override: both keys must agree, and an override can never exceed a tier ceiling |
| `settings_tests.rs` | 21 | Adjustable settings: allowed ranges and the 7-day timelock |
| `solvency_tests.rs` | 16 | Tier ceilings, the pool never commits more than it holds, loyalty points, eligibility |
| `governance_tests.rs` | 13 | Three-role governance: two of three to pass a change, 7-day delay, recovery after a lost key |
| `t2_mutation_gap_tests.rs` | 12 | More mutation-testing gap cases |
| `p2_findings_tests.rs` | 6 | Regression tests for two findings from an internal security review |
| `blend_scenario_tests.rs` | 5 | What the pool does if the Blend lending market behind the vault is exploited |
| `profiling_tests.rs` | 5 | CPU and memory cost of the heaviest calls, against Soroban's limits |
| `pool_solvency_tests.rs` | 3 | Solvency under the pool's liquidity model |
| `pool_demo_tests.rs` | 1 | One full walkthrough: stake, claim, stream, withdraw |

`common.rs` holds shared setup and has no tests of its own.

### Other contracts (62)

| Crate | Tests | What it checks |
|---|---|---|
| `cctp-adapter` | 25 | Full CCTP flows against Circle's real CCTP v2 contracts: stake, back, withdraw home, payouts home |
| `safu-account` | 16 | The account accepts only its owner's own Ethereum or Solana signature |
| `covered-registry` | 13 | One test per registry rule: a wallet binds to one stake for good, at most 3, only the writer key can add |
| `cctp-common` | 8 | CCTP message parsing |

## 2. Fuzz targets (6)

A fuzz target throws long random sequences of actions at a contract and checks after every action that the money rules still hold. Normal tests cover the cases someone thought of; fuzzing looks for the ones nobody did.

| Target | Crate | What must always hold |
|---|---|---|
| `fuzz_solvency` | protection-pool | The pool never owes more than it holds, across random stakes, withdrawals, claims, streams, cancels and time jumps |
| `fuzz_backers` | protection-pool | Every backer's money is accounted for exactly, through the full claim state machine |
| `fuzz_override` | protection-pool | The two-key override and key rotation never double-pay or leave a stale approval |
| `fuzz_registry` | covered-registry | The registry always matches a shadow model of its rules, re-read after every action |
| `fuzz_check_auth` | safu-account | Only the owner's real signature moves money: mutated, foreign or malformed signatures always fail |
| `fuzz_cctp_flows` | cctp-adapter | CCTP messages cannot be tampered with, replayed or redirected, with Circle's real contracts in the loop |

Last runs, all clean:

| Target | Runs | Date | Code |
|---|---|---|---|
| `fuzz_solvency` | 18,918 | 2026-09-29 | `4970bc7` (current) |
| `fuzz_backers` | 14,046 | 2026-09-29 | `4970bc7` (current) |
| `fuzz_override` | 14,611 | 2026-09-29 | `4970bc7` (current) |
| `fuzz_registry` | 24,850 | 2026-09-23 | before the vault rebalancing change, which does not touch the registry |
| `fuzz_check_auth` | 262,118 | 2026-09-24 | before the vault rebalancing change, which does not touch the account |
| `fuzz_cctp_flows` | 23,189 | 2026-09-24 | before the vault rebalancing change |

```bash
cd contracts/protection-pool
cargo fuzz run fuzz_solvency -- -max_total_time=300
```

Run fuzzing on a Linux server inside a Docker container with fixed CPU and memory limits. On macOS the address sanitizer crashes before any input runs.

## 3. Backend (632)

The backend lives in a private repository with the scanner and the tier engine, so its tests are listed here without the code. The suite covers:

- Claim filing end to end: one or many transactions, the 20-drain limit, the 30-day window
- Covered-wallet registry: ownership proof, the on-chain registry writer, races between two registrations
- Loss pricing: the price at the moment of loss, per asset and chain
- Wrongful-liquidation claims: reading the liquidation, checking the lender, pricing the loss
- Oracle signing: payload layout matching the contract byte for byte
- The fee relayer for Ethereum and Solana users
- The network switch: a mainnet pool refuses test settings and a testnet scanner
- Error messages: nothing internal leaks to the user

## 4. EVM (21)

```bash
cd contracts-evm
forge test
```

| File | Tests | What it checks |
|---|---|---|
| `MockLendingMarket.t.sol` | 15 | The mock lending market used to stage liquidations on Sepolia |
| `RegistrySender.t.sol` | 6 | The LayerZero registry sender (earlier design, kept for reference) |

## 5. Site (17)

```bash
npm install
npm run test     # vitest
npm run build    # type check and production build
npm run lint     # oxlint
```

`src/lib/friendly-error.test.ts` checks that contract and wallet errors turn into plain-English messages.

## 6. Live testnet end to end

These scripts run the real contracts, the real backend and real Circle CCTP against Stellar testnet, Sepolia and Solana devnet. They sit in the private repository with the backend they drive. The fast-build guard below is public.

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
