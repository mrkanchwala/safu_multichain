// SAFU pool liquidity model. One day per step, the new pool's rules as the
// contract runs them (protection-pool, audit tag audit-freeze-2026-11-18):
//   - a claim is admitted only while owed-on-claims + it <= stakes + backing,
//     within the daily intake limit and the oracle's claims-per-day limit;
//     otherwise it waits in line, strictly in order (claim.rs submit_claim)
//   - no payout on a stake younger than the time gate; approval forfeits the
//     stake; then cooldown, then even release over the payout spread
//   - each day's payout limit is shared by what each claim is still owed
//   - stakers may leave any time if there is cash; backers only take out
//     capital that claims don't need, dripped over days (backer.rs)
//   - yield on invested money is split by the stakers'/backers' share of the
//     pool times their yield share; SAFU keeps the rest (vault.rs index_bumps)
// Money is in dollars. Claimants are assumed to file, approve and collect on
// time; real ones may be slower, which only eases cash.

import { FIXED } from './settings.js';

export const DAYS_PER_YEAR = 365;
const DAYS_PER_MONTH = 30;
export const PANIC_DAYS = 60;
export const PANIC_CHURN_X = 3;

export function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Band rate (percent) for a utilisation (share owed, 0..1): calm / busy / stressed. */
export function bandPct(util, low, mid, high) {
  if (util < FIXED.utilLowPct / 100) return low;
  if (util < FIXED.utilMidPct / 100) return mid;
  return high;
}

/** Backer exit length in days at a utilisation (backer.rs drip_days). */
export function dripDays(util, minDays, maxDays) {
  const u = Math.min(Math.max(util, 0), FIXED.dripFullUtilPct / 100) / (FIXED.dripFullUtilPct / 100);
  return Math.max(1, Math.ceil(minDays + (maxDays - minDays) * u));
}

/** Most a wallet can be paid: stake x its tier multiple. */
export function ceiling(amount, tier, s) {
  return amount * ({ A: s.tierA, B: s.tierB, C: s.tierC })[tier];
}

function makePool(s, a, rand) {
  const p = {
    s, a, rand, day: 0,
    cash: 0, deployed: 0,
    staked: 0, backed: 0, pending: [], // pending backing: { amount, day }
    allocated: 0,
    stakers: [], nextId: 1,
    queue: [], claims: [], // queue: waiting claims; claims: admitted
    sIdx: 0, bIdx: 0, sReserved: 0, bReserved: 0, bOwed: 0, payable: 0, safu: 0,
    backerReq: null, // { amount, paid, start }
    dayAdmitted: 0, dayCount: 0,
    m: { cashShortDays: 0, firstShortDay: 0, exitsWaited: 0, maxQueueDays: 0, thefts: 0,
      claimsPaid: 0, paidOut: 0, stakerYield: 0, backerYield: 0, minRatio: Infinity,
      worstGap: Infinity, daysToPaid: [], paidLog: [], series: [] },
  };
  return p;
}

const capacity = (p) => p.staked + p.backed;
const util = (p) => (capacity(p) > 0 ? p.allocated / capacity(p) : 0);
const frozen = (p) => p.a.freezeDay > 0 && p.day >= p.a.freezeDay && p.day < p.a.freezeDay + p.a.freezeDays;
const panic = (p) => p.a.panicDay > 0 && p.day >= p.a.panicDay && p.day < p.a.panicDay + PANIC_DAYS;
const activeStakers = (p) => p.stakers.length;

function pickTier(p) {
  const r = p.rand() * 100;
  if (r < p.a.tierAPct) return 'A';
  if (r < p.a.tierAPct + p.a.tierBPct) return 'B';
  return 'C';
}

function stakeSize(p) {
  const lo = (p.s.poolCap * p.s.minStakePct) / 100;
  const hi = (p.s.poolCap * p.s.maxStakePct) / 100;
  return lo + (hi - lo) * p.rand();
}

/** A new stake; false if the pool is full (the whole deposit is refused). */
function stake(p, amount) {
  if (p.staked + amount > p.s.poolCap) return false;
  p.stakers.push({ id: p.nextId++, amount, tier: pickTier(p), day: p.day, idx: p.sIdx, owed: 0, claim: null });
  p.staked += amount;
  p.cash += amount;
  return true;
}

/** Today's new stake: a running budget, so small daily inflows still add whole stakes. */
function newStakers(p) {
  p.inflowCarry = (p.inflowCarry || 0) + (p.s.poolCap * p.a.inflowPctMonth) / 100 / DAYS_PER_MONTH;
  for (;;) {
    const amount = stakeSize(p);
    if (amount > p.inflowCarry || !stake(p, amount)) return;
    p.inflowCarry -= amount;
  }
}

function back(p, amount) {
  p.cash += amount;
  if (p.s.backerMaturityDays <= 0) p.backed += amount;
  else p.pending.push({ amount, day: p.day + p.s.backerMaturityDays });
}

function matureBacking(p) {
  const due = p.pending.filter((d) => d.day <= p.day);
  if (!due.length) return;
  p.pending = p.pending.filter((d) => d.day > p.day);
  for (const d of due) p.backed += d.amount;
}

/** Cash for a payment, redeeming from the lending venue if short (vault.rs pull_for_payment). */
function pull(p, amount) {
  if (p.cash >= amount) return true;
  if (!frozen(p) && p.deployed > 0) {
    const short = amount - p.cash;
    const buffer = capacity(p) * (1 - p.s.deployPct / 100);
    const take = Math.min(p.deployed, short + Math.max(buffer, 0));
    p.deployed -= take;
    p.cash += take;
  }
  return p.cash >= amount - 1e-9;
}

function markShort(p) {
  p.m.cashShortDays += 1;
  if (!p.m.firstShortDay) p.m.firstShortDay = p.day;
}

/** Idle cash above what claims owe goes to the lending venue (vault.rs push_idle). */
function invest(p) {
  if (frozen(p)) return;
  const idle = Math.max(p.cash - p.allocated, 0);
  const room = Math.max((capacity(p) * p.s.deployPct) / 100 - p.deployed, 0);
  const amount = Math.min(idle, room);
  if (amount <= 0 || amount < (capacity(p) * FIXED.autoPushMinPct) / 100) return;
  p.cash -= amount;
  p.deployed += amount;
}

/** One day of lending yield, split as vault.rs credit_yield does. */
function earn(p) {
  if (frozen(p) || p.deployed <= 0) return;
  const growth = (p.deployed * p.a.apyPct) / 100 / DAYS_PER_YEAR;
  const cap = capacity(p);
  if (cap <= 0) return;
  const toStakers = p.staked > 0 ? ((growth * p.staked) / cap) * (p.s.stakerYieldPct / 100) : 0;
  const toBackers = p.backed > 0 ? ((growth * p.backed) / cap) * (p.s.backerYieldPct / 100) : 0;
  if (p.staked > 0) p.sIdx += toStakers / p.staked;
  p.sReserved += toStakers;
  p.bReserved += toBackers;
  p.bOwed += toBackers;
  p.safu += growth - toStakers - toBackers;
  p.cash += growth; // harvested: growth leaves the venue as cash, book value stays
  p.m.stakerYield += toStakers;
  p.m.backerYield += toBackers;
}

const yieldOf = (p, st) => st.owed + st.amount * (p.sIdx - st.idx);

function admit(p, c) {
  p.allocated += c.entitlement;
  p.dayAdmitted += c.entitlement;
  if (c.counted) p.dayCount += 1;
  c.admittedDay = p.day;
  c.approveDay = Math.max(p.day, c.staker.day + p.s.timeGateDays);
  c.status = 'admitted';
  p.claims.push(c);
}

function canAdmit(p, c, checkOracle) {
  const cap = capacity(p);
  if (p.allocated + c.entitlement > cap) return false;
  const intake = (cap * bandPct(util(p), p.s.admitLowPct, p.s.admitMidPct, p.s.admitHighPct)) / 100;
  if (p.dayAdmitted + c.entitlement > intake) return false;
  if (checkOracle && p.dayCount >= Math.max(1, Math.floor(activeStakers(p) / FIXED.oracleDivisor))) return false;
  return true;
}

/** A theft on a covered wallet: the claim is filed the same day (claim.rs submit_claim). */
function theft(p, st) {
  const loss = -Math.log(1 - p.rand()) * p.a.avgLoss;
  const c = { staker: st, entitlement: Math.min(loss, ceiling(st.amount, st.tier, p.s)), streamed: 0,
    theftDay: p.day, status: 'queued', counted: true };
  st.claim = c;
  p.m.thefts += 1;
  if (p.queue.length === 0 && canAdmit(p, c, true)) admit(p, c);
  else p.queue.push(c);
}

/** First in line goes as soon as it fits; line order, never speed. */
function releaseQueue(p) {
  while (p.queue.length && canAdmit(p, p.queue[0], false)) {
    const c = p.queue.shift();
    c.counted = false;
    p.m.maxQueueDays = Math.max(p.m.maxQueueDays, p.day - c.theftDay);
    admit(p, c);
  }
  for (const c of p.queue) p.m.maxQueueDays = Math.max(p.m.maxQueueDays, p.day - c.theftDay);
}

/** Approval forfeits the stake and starts cooldown + payout spread (claim.rs activate_claim). */
function approve(p, c) {
  const st = c.staker;
  st.owed = yieldOf(p, st);
  st.idx = p.sIdx;
  p.payable += Math.min(st.owed, p.sReserved);
  p.sReserved -= Math.min(st.owed, p.sReserved);
  c.snapshot = capacity(p);
  p.staked -= st.amount;
  p.stakers = p.stakers.filter((x) => x !== st);
  c.cooldownEnd = p.day + p.s.cooldownDays;
  c.vestEnd = c.cooldownEnd + p.s.vestingDays;
  c.status = 'active';
}

/** Each active claim collects its fair share of today's payout limit (claim.rs claim_stream). */
function payClaims(p) {
  const capSnap = capacity(p);
  const allocSnap = p.allocated;
  let short = false;
  for (const c of p.claims) {
    if (c.status !== 'active' || p.day < c.cooldownEnd) continue;
    const span = c.vestEnd - c.cooldownEnd;
    const vested = span <= 0 ? c.entitlement : (c.entitlement * Math.min(p.day - c.cooldownEnd, span)) / span;
    const claimable = vested - c.streamed;
    if (claimable <= 1e-9) continue;
    const base = Math.max(capSnap, c.snapshot);
    const rate = base > 0 ? bandPct(allocSnap / base, p.s.payoutLowPct, p.s.payoutMidPct, p.s.payoutHighPct) : FIXED.emptyPoolPayoutPct;
    const remaining = c.entitlement - c.streamed;
    const share = ((base * rate) / 100) * remaining / Math.max(allocSnap, remaining);
    const amount = Math.min(claimable, share);
    if (amount <= 0) continue;
    if (!pull(p, amount)) { short = true; continue; }
    p.cash -= amount;
    c.streamed += amount;
    p.allocated -= amount;
    p.m.paidOut += amount;
    if (c.streamed >= c.entitlement - 1e-6) {
      c.status = 'done';
      p.m.claimsPaid += 1;
      p.m.daysToPaid.push(p.day - c.theftDay);
      p.m.paidLog.push({ theft: c.theftDay, staked: c.staker.day, done: p.day });
    }
  }
  if (short) markShort(p);
  p.claims = p.claims.filter((c) => c.status !== 'done');
}

/** Stakers who leave today: stake + their yield, if there is cash (stake.rs withdraw). */
function exits(p) {
  const rate = (p.a.churnPctMonth / 100 / DAYS_PER_MONTH) * (panic(p) ? PANIC_CHURN_X : 1);
  const leaving = p.stakers.filter((st) => !st.claim && p.rand() < rate);
  let short = false;
  for (const st of leaving) {
    const y = Math.min(yieldOf(p, st), p.sReserved);
    if (!pull(p, st.amount + y)) { short = true; p.m.exitsWaited += 1; continue; }
    p.cash -= st.amount + y;
    p.sReserved -= y;
    p.staked = Math.max(p.staked - st.amount, 0);
    p.stakers = p.stakers.filter((x) => x !== st);
  }
  if (short) markShort(p);
}

/** Backers' exit drips out daily, only from capital claims don't need (backer.rs). */
function backerDrip(p) {
  if (p.a.backerExitDay > 0 && p.day === p.a.backerExitDay && p.backed > 0) {
    p.backerReq = { amount: p.backed, paid: 0, start: p.day };
  }
  const r = p.backerReq;
  if (!r) return;
  const free = capacity(p) - p.allocated;
  const days = dripDays(util(p), p.s.dripMinDays, p.s.dripMaxDays);
  const unlocked = Math.min((r.amount * (p.day - r.start + 1)) / days, r.amount);
  const amount = Math.min(unlocked - r.paid, Math.max(free, 0));
  const y = Math.min(p.bOwed, p.bReserved);
  if (amount <= 0) return;
  if (!pull(p, amount + y)) { markShort(p); return; }
  p.cash -= amount + y;
  p.bOwed -= y;
  p.bReserved -= y;
  p.backed -= amount;
  r.paid += amount;
  if (r.paid >= r.amount - 1e-6) p.backerReq = null;
}

function payYieldOwed(p) {
  if (p.payable <= 0 || p.cash <= p.allocated) return;
  const amount = Math.min(p.payable, p.cash - p.allocated);
  p.cash -= amount;
  p.payable -= amount;
}

function record(p) {
  const assets = p.cash + p.deployed;
  const pendingBacking = p.pending.reduce((t, d) => t + d.amount, 0);
  const owed = p.staked + p.backed + pendingBacking + p.sReserved + p.bReserved + p.payable + p.allocated;
  const ratio = owed > 0 ? assets / owed : Infinity;
  p.m.minRatio = Math.min(p.m.minRatio, ratio);
  p.m.worstGap = Math.min(p.m.worstGap, assets - owed);
  p.m.series.push({ day: p.day, assets, owed, onClaims: p.allocated, inLine: p.queue.length, stakers: p.stakers.length });
}

/** One run. Returns the run's metrics and its daily series. */
export function simulate(s, a, seed = 1) {
  const p = makePool(s, a, rng(seed));
  while (p.staked < (s.poolCap * a.startFillPct) / 100 && stake(p, stakeSize(p)));
  if (a.backerPct > 0) back(p, (s.poolCap * a.backerPct) / 100);
  invest(p);
  const days = Math.round(a.years * DAYS_PER_YEAR);
  for (p.day = 1; p.day <= days; p.day += 1) {
    p.dayAdmitted = 0;
    p.dayCount = 0;
    matureBacking(p);
    earn(p);
    if (!panic(p)) newStakers(p);
    releaseQueue(p);
    const daily = a.theftPctYear / 100 / DAYS_PER_YEAR;
    const burst = a.burstDay > 0 && p.day === a.burstDay ? a.burstPct / 100 : 0;
    for (const st of [...p.stakers]) if (!st.claim && p.rand() < daily + burst) theft(p, st);
    for (const c of p.claims) if (c.status === 'admitted' && p.day >= c.approveDay) approve(p, c);
    payClaims(p);
    exits(p);
    backerDrip(p);
    payYieldOwed(p);
    invest(p);
    record(p);
  }
  p.m.years = a.years;
  p.m.endStakers = p.stakers.length;
  p.m.stillOwed = p.allocated;
  p.m.inLineAtEnd = p.queue.length;
  p.m.safuRevenue = p.safu;
  return p.m;
}

const median = (xs) => {
  if (!xs.length) return 0;
  const v = [...xs].sort((x, y) => x - y);
  return v[Math.floor(v.length / 2)];
};
const pct = (xs, q) => {
  if (!xs.length) return 0;
  const v = [...xs].sort((x, y) => x - y);
  return v[Math.min(v.length - 1, Math.floor(q * v.length))];
};

/** Many runs with the same settings. The page's headline numbers. */
export function runMany(s, a) {
  const runs = [];
  for (let i = 0; i < a.runs; i += 1) runs.push(simulate(s, a, i + 1));
  const onTime = runs.filter((r) => r.cashShortDays === 0).length;
  const byRatio = [...runs].sort((x, y) => x.minRatio - y.minRatio);
  return {
    runs: runs.length,
    onTimePct: (onTime / runs.length) * 100,
    worstRatio: byRatio[0].minRatio,
    typicalRatio: median(runs.map((r) => r.minRatio)),
    worstWaitInLine: Math.max(...runs.map((r) => r.maxQueueDays)),
    firstShortDay: Math.min(...runs.map((r) => r.firstShortDay || Infinity)),
    daysToPaid: median(runs.flatMap((r) => r.daysToPaid)),
    daysToPaidSlow: pct(runs.flatMap((r) => r.daysToPaid), 0.95),
    thefts: median(runs.map((r) => r.thefts)),
    paidOut: median(runs.map((r) => r.paidOut)),
    safuPerYear: median(runs.map((r) => r.safuRevenue / r.years)),
    stakerYieldPerYear: median(runs.map((r) => r.stakerYield / r.years)),
    backerYieldPerYear: median(runs.map((r) => r.backerYield / r.years)),
    typicalRun: byRatio[Math.floor(byRatio.length / 2)].series,
    worstRun: byRatio[0].series,
  };
}

/** Safe-region grid: % of runs paid on time per (theft rate, backer money) pair. */
export function safeRegion(s, a, theftRates, backerPcts, runs = 20) {
  return theftRates.map((t) => backerPcts.map((b) => {
    const r = runMany(s, { ...a, theftPctYear: t, backerPct: b, runs });
    return { theft: t, backer: b, onTimePct: r.onTimePct, worstRatio: r.worstRatio };
  }));
}
