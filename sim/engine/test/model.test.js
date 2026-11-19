// One test per pool rule the model applies, plus sanity checks on direction.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { defaults, check, SETTINGS } from '../settings.js';
import { assumptionDefaults, checkAssumptions } from '../assumptions.js';
import { simulate, runMany, bandPct, dripDays, ceiling, safeRegion } from '../model.js';

const S = defaults();
const A = { ...assumptionDefaults(), runs: 10 };

test('defaults are valid', () => {
  assert.deepEqual(check(S), []);
  assert.deepEqual(checkAssumptions(A), []);
});

test('every control has a one-line tip', () => {
  for (const s of SETTINGS) assert.ok(s.tip && !s.tip.includes('\n') && s.tip.length <= 110, s.key);
});

test('order rules are enforced (tier B above tier A is refused)', () => {
  assert.ok(check({ ...S, tierB: 16 }).length > 0);
  assert.ok(check({ ...S, payoutHighPct: 5.5 }).length > 0);
});

test('payout ceiling = stake x tier multiple', () => {
  assert.equal(ceiling(100, 'A', S), 1500);
  assert.equal(ceiling(100, 'C', { ...S, tierC: 2 }), 200);
});

test('daily limits follow the three bands', () => {
  assert.equal(bandPct(0.1, 5, 3, 1), 5);
  assert.equal(bandPct(0.2, 5, 3, 1), 3);
  assert.equal(bandPct(0.5, 5, 3, 1), 1);
});

test('backer exit length runs from min (idle) to max (half owed)', () => {
  assert.equal(dripDays(0, 7, 90), 7);
  assert.equal(dripDays(0.5, 7, 90), 90);
  assert.equal(dripDays(0.9, 7, 90), 90);
});

test('same seed, same result', () => {
  assert.deepEqual(simulate(S, A, 7).series.at(-1), simulate(S, A, 7).series.at(-1));
});

test('no claim finishes before gate + cooldown + payout spread', () => {
  const r = simulate(S, { ...A, theftPctYear: 20 }, 3);
  assert.ok(r.paidLog.length > 0);
  for (const c of r.paidLog) {
    const covered = Math.max(c.theft, c.staked + S.timeGateDays);
    assert.ok(c.done >= covered + S.cooldownDays + S.vestingDays, JSON.stringify(c));
  }
});

test('a longer payout spread pays later', () => {
  const fast = runMany({ ...S, vestingDays: 30 }, { ...A, theftPctYear: 5 });
  const slow = runMany({ ...S, vestingDays: 90 }, { ...A, theftPctYear: 5 });
  assert.ok(slow.daysToPaid > fast.daysToPaid);
});

test('owed on claims never goes above stakes + backing when admitted', () => {
  const r = simulate(S, { ...A, theftPctYear: 30, burstDay: 100, burstPct: 50 }, 2);
  for (const d of r.series) assert.ok(d.onClaims >= -1e-6);
});

test('mass theft puts claims in line instead of over-admitting', () => {
  const r = simulate(S, { ...A, burstDay: 120, burstPct: 60, backerPct: 0 }, 4);
  assert.ok(r.maxQueueDays > 0);
});

test('money is conserved: assets only change by inflows, yield and payouts', () => {
  const a = { ...A, years: 1, inflowPctMonth: 0, churnPctMonth: 0, apyPct: 0, theftPctYear: 0 };
  const r = simulate(S, a, 1);
  const first = r.series[0].assets;
  assert.ok(Math.abs(r.series.at(-1).assets - first) < 1e-6);
});

test('more thefts never make the pool safer', () => {
  const low = runMany(S, { ...A, theftPctYear: 0.5 });
  const high = runMany(S, { ...A, theftPctYear: 10 });
  assert.ok(high.typicalRatio <= low.typicalRatio);
});

test('more backer money never makes the pool less safe', () => {
  const none = runMany(S, { ...A, theftPctYear: 5, backerPct: 0 });
  const lots = runMany(S, { ...A, theftPctYear: 5, backerPct: 50 });
  assert.ok(lots.typicalRatio >= none.typicalRatio);
});

test('SAFU keeps the yield the share settings hold back', () => {
  const all = runMany({ ...S, stakerYieldPct: 100, backerYieldPct: 100 }, A);
  const eighty = runMany({ ...S, stakerYieldPct: 80, backerYieldPct: 80 }, A);
  assert.ok(all.safuPerYear < 1);
  assert.ok(eighty.safuPerYear > 0);
});

test('frozen lending venue earns nothing while frozen', () => {
  const r = runMany(S, { ...A, years: 1, freezeDay: 1, freezeDays: 365 });
  assert.equal(r.stakerYieldPerYear, 0);
});

test('safe-region grid has one cell per pair', () => {
  const g = safeRegion(S, { ...A, years: 1 }, [0.5, 5], [0, 10], 3);
  assert.equal(g.length, 2);
  assert.equal(g[0].length, 2);
});
