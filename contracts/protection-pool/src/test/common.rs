//! Shared test setup. Env and Client are kept as separate local bindings
//! at each call site (not bundled into one struct); a struct can't hold
//! both an owned Env and a Client borrowing that same Env without being
//! self-referential. `new_env()` + `setup(&env)` is the standard Soroban
//! pattern for avoiding that.

#![cfg(test)]

use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, BytesN, ConversionError, Env, InvokeError};
use std::{println, string::{String, ToString}, thread, time::Duration, vec::Vec};

use crate::error::PoolError;
use crate::types::{
    BPS_DENOMINATOR, LEDGERS_PER_DAY, MAX_STAKE_BPS, MIN_STAKE_BPS, STAKE_BPS_DENOMINATOR,
    TIME_GATE_LEDGERS,
};
use crate::{ProtectionPool, ProtectionPoolClient};

pub const STROOPS_PER_UNIT: i128 = 10_000_000;

/// 10,000 units in stroops (7dp): a clean round pool cap for test math.
pub const POOL_CAP: i128 = 100_000_000_000;
/// Derived from the real contract ratio, not a separate hardcoded literal --
/// a duplicate magic number here is exactly what let these drift out of sync
/// with `types.rs` when the ratio changed 2026-09-19 (33 tests broke on
/// `StakeOutOfRange` before this fix landed).
pub const MIN_STAKE: i128 = POOL_CAP * MIN_STAKE_BPS / STAKE_BPS_DENOMINATOR;
pub const MAX_STAKE: i128 = POOL_CAP * MAX_STAKE_BPS / STAKE_BPS_DENOMINATOR;
/// Comfortably within [MIN_STAKE, MAX_STAKE]: the default stake size
/// used by tests that don't care about boundary behavior.
pub const MID_STAKE: i128 = (MIN_STAKE + MAX_STAKE) / 2;

pub fn new_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 1_000_000;
        li.timestamp = 1_700_000_000;
    });
    env
}

pub struct Setup<'a> {
    pub client: ProtectionPoolClient<'a>,
    pub admin: Address,
    pub oracle: Address,
    pub co_signer: Address,
    /// Pre-audit hardening: the third governance role.
    pub guardian: Address,
    pub token_admin: StellarAssetClient<'a>,
    pub token_id: Address,
    /// D1 (T2): needed because the approval payload commits to
    /// `env.current_contract_address()`. A test can only reproduce that
    /// binding by running the payload builder inside `env.as_contract(...)`,
    /// which needs the id.
    pub contract_id: Address,
    /// D1 (T2): the oracle's Ed25519 ATTESTATION key, the private half of
    /// what `initialize` stored as `OraclePubKey`. Distinct from
    /// `Setup::oracle`, which is the policy `Address`; the two identities
    /// are deliberately separate (see storage::DataKey::OraclePubKey).
    pub oracle_key: SigningKey,
}

pub fn setup(env: &Env) -> Setup<'_> {
    setup_with_cap(env, POOL_CAP)
}

pub fn setup_with_cap(env: &Env, pool_cap: i128) -> Setup<'_> {
    let admin = Address::generate(env);
    let oracle = Address::generate(env);
    let co_signer = Address::generate(env);
    let guardian = Address::generate(env);

    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token_id = sac.address();
    let token_admin = StellarAssetClient::new(env, &token_id);

    let oracle_key = oracle_signing_key();
    // CHANGED 2026-08-17 (7a audit, Finding 3): the pool is now configured by
    // `__constructor` at registration rather than by a separate
    // `client.initialize(...)` call, matching how it will actually deploy.
    let contract_id = env.register(
        ProtectionPool,
        (
            admin.clone(),
            oracle.clone(),
            verifying_key_bytes(env, &oracle_key),
            co_signer.clone(),
            guardian.clone(),
            token_id.clone(),
            pool_cap,
        ),
    );
    let client = ProtectionPoolClient::new(env, &contract_id);

    Setup {
        client,
        admin,
        oracle,
        co_signer,
        guardian,
        token_admin,
        token_id,
        contract_id,
        oracle_key,
    }
}

// -----------------------------------------------------------------------
// D1 (T2), oracle approval signing helpers.
//
// These sign REAL Ed25519 approvals against the same payload builder the
// contract verifies with, rather than stubbing verification out. That is
// the point: `submit_claim`'s oracle path is now a cryptographic gate, and
// a test suite that bypassed it would no longer be testing the contract
// that ships.
//
// What this does NOT prove is that the payload ENCODING is what the
// off-chain signer produces: sharing `build_approval_payload` with the
// contract makes that self-consistent by construction. Two separate things
// cover it: `payload_encoding_is_stable_and_field_ordered` below (an
// independently written, longhand reconstruction, so a reordered or dropped
// field fails) and, cross-language, the `api/signer.py` KMS round-trip.
// -----------------------------------------------------------------------

/// Fixed key, not randomly generated, a failing signature test must be
/// reproducible from the source alone, and a random key would make a
/// failure depend on the run.
pub fn oracle_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// A DIFFERENT valid key, for "signed by the wrong party" tests.
pub fn other_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[9u8; 32])
}

pub fn verifying_key_bytes(env: &Env, sk: &SigningKey) -> BytesN<32> {
    BytesN::from_array(env, &sk.verifying_key().to_bytes())
}

/// One hour out: comfortably inside `MAX_APPROVAL_WINDOW_SECONDS` (24h),
/// so the default path exercises a normal approval rather than a boundary.
pub fn default_deadline(env: &Env) -> u64 {
    env.ledger().timestamp() + 3_600
}

/// Signs an already-built payload. Exposed separately so a test can sign a
/// deliberately WRONG payload (e.g. one carrying V8's domain string) that
/// `build_approval_payload` would never produce.
pub fn sign_bytes(env: &Env, sk: &SigningKey, payload: &soroban_sdk::Bytes) -> BytesN<64> {
    let bytes: Vec<u8> = payload.iter().collect();
    BytesN::from_array(env, &sk.sign(&bytes).to_bytes())
}

/// Rebuilds the exact payload the contract will build, then signs it with
/// `sk`. `env.as_contract` is required, not incidental: the payload commits
/// to `env.current_contract_address()`, which only resolves inside the
/// contract's own context.
#[allow(clippy::too_many_arguments)]
pub fn sign_approval_with(
    env: &Env,
    s: &Setup,
    sk: &SigningKey,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    entitlement: &i128,
    tier: &u32,
    hack_timestamp: &u64,
    deadline: &u64,
) -> BytesN<64> {
    let payload = env.as_contract(&s.contract_id, || {
        crate::claim::build_approval_payload(
            env,
            wallet,
            tx_hash,
            *entitlement,
            *tier,
            *hack_timestamp,
            *deadline,
        )
    });
    sign_bytes(env, sk, &payload)
}

/// Signs with the configured oracle key, the happy path.
#[allow(clippy::too_many_arguments)]
pub fn sign_approval(
    env: &Env,
    s: &Setup,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    entitlement: &i128,
    tier: &u32,
    hack_timestamp: &u64,
    deadline: &u64,
) -> BytesN<64> {
    sign_approval_with(
        env,
        s,
        &s.oracle_key.clone(),
        wallet,
        tx_hash,
        entitlement,
        tier,
        hack_timestamp,
        deadline,
    )
}

/// Drop-in for the pre-D1 `s.client.submit_claim(..)`: same six arguments,
/// with a valid deadline and signature generated to match. Every existing
/// test call site was migrated to this, so those tests keep asserting the
/// behaviour they were written for while now also traversing the real
/// signature gate. Signature-specific behaviour is tested directly against
/// `s.client.submit_claim` in `claim_tests.rs`, not through this helper.
#[allow(clippy::too_many_arguments)]
pub fn submit_claim_signed(
    env: &Env,
    s: &Setup,
    caller: &Address,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    entitlement: &i128,
    tier: &u32,
    hack_timestamp: &u64,
) -> BytesN<32> {
    let deadline = default_deadline(env);
    let sig = sign_approval(
        env,
        s,
        wallet,
        tx_hash,
        entitlement,
        tier,
        hack_timestamp,
        &deadline,
    );
    s.client.submit_claim(
        caller,
        wallet,
        tx_hash,
        entitlement,
        tier,
        hack_timestamp,
        &deadline,
        &sig,
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn try_submit_claim_signed(
    env: &Env,
    s: &Setup,
    caller: &Address,
    wallet: &Address,
    tx_hash: &BytesN<32>,
    entitlement: &i128,
    tier: &u32,
    hack_timestamp: &u64,
) -> Result<Result<BytesN<32>, ConversionError>, Result<PoolError, InvokeError>> {
    let deadline = default_deadline(env);
    let sig = sign_approval(
        env,
        s,
        wallet,
        tx_hash,
        entitlement,
        tier,
        hack_timestamp,
        &deadline,
    );
    s.client.try_submit_claim(
        caller,
        wallet,
        tx_hash,
        entitlement,
        tier,
        hack_timestamp,
        &deadline,
        &sig,
    )
}

pub fn new_funded_address(env: &Env, s: &Setup, funding: i128) -> Address {
    let addr = Address::generate(env);
    s.token_admin.mint(&addr, &funding);
    addr
}

/// Stakes MID_STAKE from a freshly funded, freshly generated staker with
/// a freshly generated beneficiary. Returns (staker, beneficiary), the
/// shape most claim/stream tests start from.
pub fn staked_wallet(env: &Env, s: &Setup) -> (Address, Address) {
    staked_wallet_amount(env, s, MID_STAKE)
}

pub fn staked_wallet_amount(env: &Env, s: &Setup, amount: i128) -> (Address, Address) {
    let staker = new_funded_address(env, s, amount);
    let beneficiary = Address::generate(env);
    s.client.stake(&staker, &amount, &beneficiary);
    (staker, beneficiary)
}

/// Seconds the test ledger advances per ledger (Stellar closes a ledger about
/// every 5s). Named so a test can convert a window in seconds into ledgers
/// without repeating the literal.
pub const SECONDS_PER_LEDGER: u64 = 5;

/// r3 (CSO M2): moves only the clock (not the ledger sequence, so no claim
/// or cooldown timer moves) far enough for `harvest`'s daily growth limit to
/// allow `growth_bps` of growth over book value.
pub fn let_growth_through(env: &Env, growth_bps: i128) {
    let days = (growth_bps / crate::types::HARVEST_MAX_GROWTH_BPS_PER_DAY + 1) as u64;
    env.ledger().with_mut(|li| li.timestamp += days * crate::types::SECONDS_PER_DAY);
}

pub fn advance_ledgers(env: &Env, n: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number += n;
        li.timestamp += (n as u64) * SECONDS_PER_LEDGER;
    });
}

/// `amount * bps / 10_000`, using the contract's own denominator. For tests
/// that express a scenario as "X% of the pool" without a hand-typed result.
pub fn bps_of(amount: i128, bps: i128) -> i128 {
    amount * bps / BPS_DENOMINATOR
}

// Independent oracles for the contract's utilisation bands. These are the
// spec, written out in the test crate on purpose: a test that asked the
// contract for its own cap would pass under any mutation of that function,
// which is exactly what the boundary tests exist to catch. The contract keeps
// these edges inline in `claim.rs` (`stress_cap`, `dynamic_outflow_bps`), so
// there is no exported constant to derive from.

/// Utilisation edges shared by the admission stress cap and the payout outflow cap.
pub const BAND_1_UTILISATION_BPS: i128 = 2_000;
pub const BAND_2_UTILISATION_BPS: i128 = 5_000;
/// Utilisation at which the lowest stress-cap band's cap equals the solvency headroom.
pub const EXACT_FILL_UTILISATION_BPS: i128 = 9_700;
/// Admission stress cap, as bps of total staked, in each band.
pub const STRESS_RATE_BAND_1_BPS: i128 = 2_500;
pub const STRESS_RATE_BAND_2_BPS: i128 = 1_000;
pub const STRESS_RATE_BAND_3_BPS: i128 = 300;
/// Payout outflow cap, as bps of the cap base, in each band.
pub const OUTFLOW_RATE_BAND_1_BPS: i128 = 500;
pub const OUTFLOW_RATE_BAND_2_BPS: i128 = 300;
pub const OUTFLOW_RATE_BAND_3_BPS: i128 = 100;

/// What the admission stress cap should be for a given pool state.
pub fn stress_cap_oracle(total_staked: i128, total_allocated: i128) -> i128 {
    let utilisation_bps = total_allocated * BPS_DENOMINATOR / total_staked;
    let rate_bps = if utilisation_bps < BAND_1_UTILISATION_BPS {
        STRESS_RATE_BAND_1_BPS
    } else if utilisation_bps < BAND_2_UTILISATION_BPS {
        STRESS_RATE_BAND_2_BPS
    } else {
        STRESS_RATE_BAND_3_BPS
    };
    bps_of(total_staked, rate_bps)
}

/// `stress_cap_oracle` applied to the pool's current state.
pub fn expected_stress_cap(s: &Setup<'_>) -> i128 {
    stress_cap_oracle(s.client.get_total_staked(), s.client.get_total_allocated())
}

/// What the payout outflow cap should be, for a utilisation measured against `base`.
pub fn outflow_cap_oracle(base: i128, total_allocated: i128) -> i128 {
    let utilisation_bps = total_allocated * BPS_DENOMINATOR / base;
    let rate_bps = if utilisation_bps < BAND_1_UTILISATION_BPS {
        OUTFLOW_RATE_BAND_1_BPS
    } else if utilisation_bps < BAND_2_UTILISATION_BPS {
        OUTFLOW_RATE_BAND_2_BPS
    } else {
        OUTFLOW_RATE_BAND_3_BPS
    };
    bps_of(base, rate_bps)
}

/// Points accrue by calendar day, not by ledger, so tests that need a non-zero
/// points balance stake for the first points tier (in days), which is
/// unrelated to the claim time gate.
pub const POINTS_TIER_1_DAYS: u32 = 90;
/// Points earned per day in the first and second tiers, before the stake-size factor.
pub const POINTS_PER_DAY_TIER_1: i128 = 100;
pub const POINTS_PER_DAY_TIER_2: i128 = 120;

/// Lands the pool inside the claim time gate.
pub fn advance_past_time_gate(env: &Env) {
    advance_ledgers(env, TIME_GATE_LEDGERS);
}

pub fn advance_days(env: &Env, days: u32) {
    advance_ledgers(env, days * LEDGERS_PER_DAY);
}

pub fn tx_hash(env: &Env, seed: u8) -> BytesN<32> {
    BytesN::from_array(env, &[seed; 32])
}

pub fn now_ts(env: &Env) -> u64 {
    env.ledger().timestamp()
}

// -----------------------------------------------------------------------
// Presentation helpers -- shared by pool_demo_tests.rs and
// blend_scenario_tests.rs's video-recordable demo tests. Not used by the
// plain assertion-only unit tests, which don't need readable output.
// -----------------------------------------------------------------------

/// Paces printed output for video recording -- without this, `cargo test
/// -- --nocapture` dumps the whole lifecycle instantly, too fast to
/// follow on screen.
pub fn pause() {
    thread::sleep(Duration::from_millis(500));
}

/// Whole-unit, comma-separated amount for readable presentation output
/// (e.g. `500_000_000_000` stroops -> "50,000"). Demo amounts are round
/// multiples of one unit, so integer division here is exact.
pub fn fmt_amount(stroops: i128) -> String {
    let whole = stroops / STROOPS_PER_UNIT;
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    grouped.chars().rev().collect()
}

/// Real Tranche 1 deploy value (README "Deploy-time arguments"): 600,000
/// XLM, matching the live testnet contract's actual pool cap. Used by
/// video-recordable demo tests so their numbers match the real
/// deployment, not the smaller default `POOL_CAP` other unit tests use.
pub const DEMO_POOL_CAP: i128 = 6_000_000_000_000;

pub fn max_stake(pool_cap: i128) -> i128 {
    pool_cap * MAX_STAKE_BPS / STAKE_BPS_DENOMINATOR
}

pub fn print_solvency(s: &Setup<'_>, label: &str) {
    let staked = s.client.get_total_staked();
    let allocated = s.client.get_total_allocated();
    println!(
        "         Pool solvency [{}]: allocated {} USDC <= staked {} USDC -> {}",
        label,
        fmt_amount(allocated),
        fmt_amount(staked),
        if allocated <= staked { "OK" } else { "VIOLATION" }
    );
    pause();
}

/// v1 (2026-09-22): change an adjustable setting the only way production can:
/// admin proposes, co-signer approves the same value, 7 days pass, execute.
pub fn set_setting_via_timelock(
    env: &Env,
    s: &Setup<'_>,
    key: crate::settings::SettingKey,
    value: i128,
) {
    s.client.propose_setting(&key, &value);
    s.client.approve_setting(&key, &value);
    let ledgers = (crate::settings::SETTINGS_TIMELOCK_SECONDS / SECONDS_PER_LEDGER) as u32 + 1;
    advance_ledgers(env, ledgers);
    s.client.execute_setting(&key);
}

/// Pre-audit hardening: make a governance change the only way production can:
/// one role proposes, a second approves the same change, 7 days pass, anyone
/// executes. Picks two roles that are not the one being replaced, read live
/// (a test may already have rotated one).
pub fn gov_apply(env: &Env, s: &Setup<'_>, change: crate::GovChange) {
    use crate::{GovChange, GovKind};
    let (admin, co, guardian) = (s.client.get_admin(), s.client.get_co_signer(), s.client.get_guardian());
    let (kind, a, b) = match &change {
        GovChange::Admin(_) => (GovKind::Admin, co, guardian),
        GovChange::CoSigner(_) => (GovKind::CoSigner, admin, guardian),
        GovChange::Guardian(_) => (GovKind::Guardian, admin, co),
        GovChange::Oracle(_, _) => (GovKind::Oracle, admin, co),
        GovChange::Vault(_) => (GovKind::Vault, admin, co),
        GovChange::Treasury(_) => (GovKind::Treasury, admin, co),
        GovChange::DeployBps(_) => (GovKind::DeployBps, admin, co),
        GovChange::RegistryWriter(_, _) => (GovKind::RegistryWriter, admin, co),
    };
    s.client.propose_change(&a, &change);
    s.client.approve_change(&b, &change);
    let ledgers = (crate::governance::GOV_DELAY_SECONDS / SECONDS_PER_LEDGER) as u32 + 1;
    advance_ledgers(env, ledgers);
    s.client.execute_change(&kind);
}
