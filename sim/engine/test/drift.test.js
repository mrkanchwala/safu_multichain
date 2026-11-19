// Fails if the model's setting defaults or bounds drift from the contract.
// Reads the Rust source directly: types.rs (defaults), settings.rs (bounds),
// points.rs (points unit).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { SETTINGS, FIXED } from '../settings.js';

const SRC = fileURLToPath(new URL('../../../contracts/protection-pool/src/', import.meta.url));
const read = (f) => readFileSync(SRC + f, 'utf8');
const RUST = [read('types.rs'), read('settings.rs'), read('points.rs')].join('\n');

const consts = {};
for (const m of RUST.matchAll(/(?:pub )?const ([A-Z0-9_]+): [a-z0-9]+ =\s*([^;]+);/g)) consts[m[1]] = m[2];

function value(name, depth = 0) {
  assert.ok(depth < 10, `cycle at ${name}`);
  const expr = consts[name];
  assert.ok(expr !== undefined, `constant ${name} not found in the contract`);
  const clean = expr.replace(/\/\/.*$/gm, '').replace(/as [iu]\d+/g, '').replace(/crate::\w+::/g, '');
  return clean.split('*').map((t) => t.trim().replace(/[()]/g, ''))
    .map((t) => (/^[\d_]+$/.test(t) ? Number(t.replace(/_/g, '')) : value(t, depth + 1)))
    .reduce((x, y) => x * y, 1);
}

const ledgerDays = (n) => value(n) / value('LEDGERS_PER_DAY');
const secDays = (n) => value(n) / value('SECONDS_PER_DAY');
const pct = (n) => value(n) / 100;
const rate = (n) => value(n) / value('POINTS_RATE_ONE');

// key -> [default, min, max] as the contract has them, in page units.
const CONTRACT = {
  tierA: [value('TIER_A_RATIO'), value('TIER_RATIO_MIN'), value('TIER_RATIO_MAX')],
  tierB: [value('TIER_B_RATIO'), value('TIER_RATIO_MIN'), value('TIER_RATIO_MAX')],
  tierC: [value('TIER_C_RATIO'), value('TIER_RATIO_MIN'), value('TIER_RATIO_MAX')],
  minStakePct: [pct('MIN_STAKE_BPS'), pct('STAKE_BPS_MIN'), pct('STAKE_BPS_MAX')],
  maxStakePct: [pct('MAX_STAKE_BPS'), pct('STAKE_BPS_MIN'), pct('STAKE_BPS_MAX')],
  timeGateDays: [ledgerDays('TIME_GATE_LEDGERS'), ledgerDays('TIME_GATE_MIN'), ledgerDays('TIME_GATE_MAX')],
  cooldownDays: [ledgerDays('COOLDOWN_LEDGERS'), ledgerDays('COOLDOWN_MIN'), ledgerDays('COOLDOWN_MAX')],
  vestingDays: [ledgerDays('VESTING_LEDGERS'), ledgerDays('VESTING_MIN'), ledgerDays('VESTING_MAX')],
  claimWindowDays: [secDays('CLAIM_WINDOW_SECONDS'), secDays('CLAIM_WINDOW_MIN'), secDays('CLAIM_WINDOW_MAX')],
  approveWindowDays: [ledgerDays('APPROVE_WINDOW_LEDGERS'), secDays('CLAIM_CLOCK_MIN'), secDays('CLAIM_CLOCK_MAX')],
  inactivityDays: [ledgerDays('COLLECTION_INACTIVITY_LEDGERS'), secDays('CLAIM_CLOCK_MIN'), secDays('CLAIM_CLOCK_MAX')],
  payoutLowPct: [pct('PAYOUT_LOW_BPS'), pct('PAYOUT_BPS_MIN'), pct('PAYOUT_BPS_MAX')],
  payoutMidPct: [pct('PAYOUT_MID_BPS'), pct('PAYOUT_BPS_MIN'), pct('PAYOUT_BPS_MAX')],
  payoutHighPct: [pct('PAYOUT_HIGH_BPS'), pct('PAYOUT_BPS_MIN'), pct('PAYOUT_BPS_MAX')],
  admitLowPct: [pct('ADMIT_LOW_BPS'), pct('ADMIT_BPS_MIN'), pct('ADMIT_BPS_MAX')],
  admitMidPct: [pct('ADMIT_MID_BPS'), pct('ADMIT_BPS_MIN'), pct('ADMIT_BPS_MAX')],
  admitHighPct: [pct('ADMIT_HIGH_BPS'), pct('ADMIT_BPS_MIN'), pct('ADMIT_BPS_MAX')],
  stakerYieldPct: [pct('YIELD_SPLIT_STAKER_BPS'), 0, pct('YIELD_BPS_MAX')],
  backerYieldPct: [pct('YIELD_SPLIT_BACKER_BPS'), 0, pct('YIELD_BPS_MAX')],
  backerMaturityDays: [secDays('BACKER_MATURITY_SECONDS'), 0, secDays('BACKER_MATURITY_MAX')],
  dripMinDays: [secDays('DRIP_MIN_SECONDS'), secDays('DRIP_MIN_MIN'), secDays('DRIP_MIN_MAX')],
  dripMaxDays: [secDays('DRIP_MAX_SECONDS'), secDays('DRIP_MAX_MIN'), secDays('DRIP_MAX_MAX')],
  pauseMaxDays: [secDays('PAUSE_MAX_SECONDS'), secDays('PAUSE_MAX_MIN'), secDays('PAUSE_MAX_MAX')],
  pauseGapDays: [secDays('PAUSE_GAP_SECONDS'), secDays('PAUSE_GAP_MIN'), secDays('PAUSE_GAP_MAX')],
  pointsStaker: [1, 0, rate('POINTS_RATE_MAX')],
  pointsBacker: [1, 0, rate('POINTS_RATE_MAX')],
};

for (const [key, [def, min, max]] of Object.entries(CONTRACT)) {
  test(`${key} matches the contract`, () => {
    const s = SETTINGS.find((x) => x.key === key);
    assert.ok(s, `${key} missing from settings.js`);
    assert.deepEqual([s.def, s.min, s.max], [def, min, max]);
  });
}

test('every SettingKey in the contract is on the page', () => {
  const block = read('settings.rs').match(/pub enum SettingKey \{([\s\S]*?)\n\}/)[1];
  const keys = [...block.matchAll(/^\s+(\w+) = \d+,/gm)].map((m) => m[1]);
  // PoolCap is page-ranged (the contract only allows raising it);
  // BackerNoticeSeconds is unused since the drip replaced it.
  const notOnPage = new Set(['PoolCap', 'BackerNoticeSeconds']);
  assert.equal(keys.length - notOnPage.size, Object.keys(CONTRACT).length);
});

test('invest share ceiling matches the contract', () => {
  assert.equal(SETTINGS.find((x) => x.key === 'deployPct').max, pct('MAX_DEPLOY_BPS'));
});

test('fixed constants match the contract', () => {
  assert.equal(FIXED.utilLowPct, pct('UTIL_LOW_BPS'));
  assert.equal(FIXED.utilMidPct, pct('UTIL_MID_BPS'));
  assert.equal(FIXED.dripFullUtilPct, pct('DRIP_FULL_UTIL_BPS'));
  assert.equal(FIXED.oracleDivisor, value('ORACLE_DAILY_LIMIT_DIVISOR'));
  assert.equal(FIXED.autoPushMinPct, pct('AUTO_PUSH_MIN_BPS'));
  assert.equal(FIXED.emptyPoolPayoutPct, pct('PAYOUT_EMPTY_POOL_BPS'));
});
