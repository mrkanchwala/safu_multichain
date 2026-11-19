// Every adjustable setting of the new pool (protection-pool, audit tag
// audit-freeze-2026-11-18), in page units. Defaults and bounds mirror
// contracts/protection-pool/src/settings.rs + types.rs; test/drift.test.js
// fails if they ever disagree. Each change on the live pool needs two signers
// and a 7-day public delay.
//
// `liquidity: false` = the setting exists but does not move cash in this
// model (claimants are assumed to file, approve and collect on time).

export const SETTINGS = [
  { key: 'tierA', group: 'Cover', label: 'Payout multiple, tier A', unit: 'x', def: 15, min: 1, max: 20, step: 1, liquidity: true,
    tip: 'Most a tier A wallet can be paid, as a multiple of its stake.' },
  { key: 'tierB', group: 'Cover', label: 'Payout multiple, tier B', unit: 'x', def: 10, min: 1, max: 20, step: 1, liquidity: true,
    tip: 'Most a tier B wallet can be paid, as a multiple of its stake.' },
  { key: 'tierC', group: 'Cover', label: 'Payout multiple, tier C', unit: 'x', def: 5, min: 1, max: 20, step: 1, liquidity: true,
    tip: 'Most a tier C wallet can be paid, as a multiple of its stake.' },

  { key: 'poolCap', group: 'Pool', label: 'Pool cap', unit: '$', def: 100000, min: 1000, max: 100000000, step: 1000, liquidity: true,
    tip: 'Most stake the pool accepts. Backing can go above it.' },
  { key: 'minStakePct', group: 'Pool', label: 'Smallest stake', unit: '% of cap', def: 0.01, min: 0.01, max: 1.25, step: 0.01, liquidity: true,
    tip: 'Smallest stake, as a share of the pool cap.' },
  { key: 'maxStakePct', group: 'Pool', label: 'Largest stake', unit: '% of cap', def: 0.1, min: 0.01, max: 1.25, step: 0.01, liquidity: true,
    tip: 'Largest stake. Anything above it is kept as backing, never lost to a claim.' },
  { key: 'deployPct', group: 'Pool', label: 'Share invested for yield', unit: '%', def: 80, min: 0, max: 80, step: 5, liquidity: true,
    tip: 'Part of the pool lent out for yield. The rest stays as cash. Never above 80%.' },

  { key: 'timeGateDays', group: 'Claim timing', label: 'Wait before a stake is covered', unit: 'days', def: 90, min: 90, max: 365, step: 1, liquidity: true,
    tip: 'A stake younger than this is not paid yet. A claim filed earlier waits, then pays in full.' },
  { key: 'cooldownDays', group: 'Claim timing', label: 'Cooldown', unit: 'days', def: 7, min: 7, max: 30, step: 1, liquidity: true,
    tip: 'Days between an approved claim and its first payment.' },
  { key: 'vestingDays', group: 'Claim timing', label: 'Payout spread', unit: 'days', def: 45, min: 30, max: 90, step: 1, liquidity: true,
    tip: 'A payout is released evenly over this many days.' },
  { key: 'claimWindowDays', group: 'Claim timing', label: 'Time to file a claim', unit: 'days', def: 30, min: 14, max: 90, step: 1, liquidity: false,
    tip: 'A claim must be filed within this many days of the theft.' },
  { key: 'approveWindowDays', group: 'Claim timing', label: 'Time to approve a claim', unit: 'days', def: 100, min: 30, max: 365, step: 1, liquidity: false,
    tip: 'Days the staker has to approve their claim.' },
  { key: 'inactivityDays', group: 'Claim timing', label: 'Time to collect', unit: 'days', def: 100, min: 30, max: 365, step: 1, liquidity: false,
    tip: 'A payout left uncollected this long returns to the pool.' },

  { key: 'payoutLowPct', group: 'Daily limits', label: 'Daily payout limit, calm pool', unit: '% of pool', def: 5, min: 0.01, max: 6, step: 0.01, liquidity: true,
    tip: 'Most paid out per day while under 20% of the pool is owed to claims.' },
  { key: 'payoutMidPct', group: 'Daily limits', label: 'Daily payout limit, busy pool', unit: '% of pool', def: 3, min: 0.01, max: 6, step: 0.01, liquidity: true,
    tip: 'Most paid out per day while 20% to 50% of the pool is owed to claims.' },
  { key: 'payoutHighPct', group: 'Daily limits', label: 'Daily payout limit, stressed pool', unit: '% of pool', def: 1, min: 0.01, max: 6, step: 0.01, liquidity: true,
    tip: 'Most paid out per day while 50% or more of the pool is owed to claims.' },
  { key: 'admitLowPct', group: 'Daily limits', label: 'New claims per day, calm pool', unit: '% of pool', def: 25, min: 0.01, max: 25, step: 0.01, liquidity: true,
    tip: 'Most new claim value accepted per day while under 20% is owed. The rest waits in line.' },
  { key: 'admitMidPct', group: 'Daily limits', label: 'New claims per day, busy pool', unit: '% of pool', def: 10, min: 0.01, max: 25, step: 0.01, liquidity: true,
    tip: 'Same, while 20% to 50% of the pool is owed.' },
  { key: 'admitHighPct', group: 'Daily limits', label: 'New claims per day, stressed pool', unit: '% of pool', def: 3, min: 0.01, max: 25, step: 0.01, liquidity: true,
    tip: 'Same, while 50% or more of the pool is owed.' },

  { key: 'stakerYieldPct', group: 'Yield', label: 'Yield share to stakers', unit: '%', def: 100, min: 0, max: 100, step: 1, liquidity: true,
    tip: 'Share of the yield on stakers’ money paid to stakers. SAFU keeps the rest.' },
  { key: 'backerYieldPct', group: 'Yield', label: 'Yield share to backers', unit: '%', def: 100, min: 0, max: 100, step: 1, liquidity: true,
    tip: 'Share of the yield on backers’ money paid to backers. SAFU keeps the rest.' },

  { key: 'backerMaturityDays', group: 'Backers', label: 'Wait before backing counts', unit: 'days', def: 0, min: 0, max: 7, step: 1, liquidity: true,
    tip: 'Days before new backing counts toward the pool.' },
  { key: 'dripMinDays', group: 'Backers', label: 'Backer exit, calm pool', unit: 'days', def: 7, min: 1, max: 90, step: 1, liquidity: true,
    tip: 'Days a backer’s exit is spread over when no claims are open.' },
  { key: 'dripMaxDays', group: 'Backers', label: 'Backer exit, stressed pool', unit: 'days', def: 90, min: 7, max: 180, step: 1, liquidity: true,
    tip: 'Days a backer’s exit is spread over when half the pool is owed to claims.' },

  { key: 'pauseMaxDays', group: 'Safety', label: 'Longest pause', unit: 'days', def: 30, min: 1, max: 30, step: 1, liquidity: false,
    tip: 'Longest the pool can be paused at once. No clock runs against anyone while paused.' },
  { key: 'pauseGapDays', group: 'Safety', label: 'Gap between pauses', unit: 'days', def: 30, min: 30, max: 90, step: 1, liquidity: false,
    tip: 'Days that must pass before another pause.' },
  { key: 'pointsStaker', group: 'Points', label: 'Points per $ per day, stakers', unit: 'pts', def: 1, min: 0, max: 10, step: 0.1, liquidity: false,
    tip: 'Points a staker earns per dollar per day.' },
  { key: 'pointsBacker', group: 'Points', label: 'Points per $ per day, backers', unit: 'pts', def: 1, min: 0, max: 10, step: 0.1, liquidity: false,
    tip: 'Points a backer earns per dollar per day.' },
];

// Fixed in the contract, not settings.
export const FIXED = {
  utilLowPct: 20, // below this share owed: calm band
  utilMidPct: 50, // below this: busy band; at or above: stressed
  dripFullUtilPct: 50,
  oracleDivisor: 10, // claims admitted per day <= max(1, stakers / 10)
  autoPushMinPct: 1, // idle cash is invested only in chunks >= 1% of the pool
  emptyPoolPayoutPct: 1,
};

// Same pairs the contract enforces at execution (settings.rs consistent_with_live).
const ORDER = [
  ['minStakePct', '<=', 'maxStakePct'],
  ['tierB', '<=', 'tierA'], ['tierC', '<=', 'tierB'],
  ['payoutMidPct', '<=', 'payoutLowPct'], ['payoutHighPct', '<=', 'payoutMidPct'],
  ['admitMidPct', '<=', 'admitLowPct'], ['admitHighPct', '<=', 'admitMidPct'],
  ['dripMinDays', '<=', 'dripMaxDays'],
  ['pauseMaxDays', '<=', 'pauseGapDays'], ['claimWindowDays', '<=', 'pauseGapDays'],
];

export function defaults() {
  return Object.fromEntries(SETTINGS.map((s) => [s.key, s.def]));
}

/** Problems with a settings object, as short sentences. Empty = valid. */
export function check(values) {
  const out = [];
  for (const s of SETTINGS) {
    const v = values[s.key];
    if (typeof v !== 'number' || Number.isNaN(v)) out.push(`${s.label}: missing`);
    else if (v < s.min || v > s.max) out.push(`${s.label}: must be ${s.min} to ${s.max} ${s.unit}`);
  }
  for (const [a, , b] of ORDER) {
    if (values[a] > values[b]) out.push(`${label(a)} can't be above ${label(b)}`);
  }
  return out;
}

function label(key) {
  return SETTINGS.find((s) => s.key === key).label;
}
