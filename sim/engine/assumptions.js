// What the model has to assume about the world. Not pool settings: nobody
// controls these. Each has a source or says it is a stress case.

export const ASSUMPTIONS = [
  { key: 'years', group: 'Run', label: 'Years simulated', unit: 'years', def: 2, min: 1, max: 10, step: 1,
    tip: 'How long each simulated run lasts.' },
  { key: 'runs', group: 'Run', label: 'Runs', unit: 'runs', def: 50, min: 10, max: 500, step: 10,
    tip: 'Each run draws thefts at random. More runs = steadier results.' },

  { key: 'startFillPct', group: 'Stakers', label: 'Pool filled at start', unit: '% of cap', def: 60, min: 1, max: 100, step: 1,
    tip: 'How full the pool is on day 1.' },
  { key: 'inflowPctMonth', group: 'Stakers', label: 'New stake per month', unit: '% of cap', def: 5, min: 0, max: 50, step: 1,
    tip: 'New stakers joining each month, as a share of the pool cap.' },
  { key: 'churnPctMonth', group: 'Stakers', label: 'Stakers leaving per month', unit: '%', def: 5, min: 0, max: 50, step: 1,
    tip: 'Share of stakers who withdraw each month. DeFi apps see under 5% after month 3 (Formo).' },
  { key: 'tierAPct', group: 'Stakers', label: 'Tier A wallets', unit: '%', def: 33, min: 0, max: 100, step: 1,
    tip: 'Share of stakers in tier A. No public data; even split by default.' },
  { key: 'tierBPct', group: 'Stakers', label: 'Tier B wallets', unit: '%', def: 33, min: 0, max: 100, step: 1,
    tip: 'Share of stakers in tier B. The rest are tier C.' },

  { key: 'theftPctYear', group: 'Thefts', label: 'Stakers robbed per year', unit: '%', def: 0.5, min: 0, max: 30, step: 0.05,
    tip: '0.15–0.35% of crypto users lost funds in 2025 (Chainalysis, Scam Sniffer). Default is above that.' },
  { key: 'avgLoss', group: 'Thefts', label: 'Average amount stolen', unit: '$', def: 2500, min: 10, max: 100000, step: 10,
    tip: 'Average wallet theft is $790–$4,500 (DeepStrike 2025). Payouts never exceed the wallet’s ceiling.' },

  { key: 'backerPct', group: 'Backers', label: 'Backer money', unit: '% of cap', def: 10, min: 0, max: 200, step: 1,
    tip: 'Money put in to back the pool. It is never lost to a single claim.' },
  { key: 'backerExitDay', group: 'Backers', label: 'Backers ask to leave on day', unit: 'day', def: 0, min: 0, max: 3650, step: 1,
    tip: '0 = backers stay. Otherwise all backing asks to leave on this day.' },

  { key: 'apyPct', group: 'Yield', label: 'Lending yield', unit: '% a year', def: 5, min: 0, max: 20, step: 0.1,
    tip: 'Yield on invested money. Stellar USDC lending paid about 5–7% in 2026; it moves.' },

  { key: 'panicDay', group: 'Stress tests', label: 'Panic starts on day', unit: 'day', def: 0, min: 0, max: 3650, step: 1,
    tip: '0 = off. From this day, for 60 days: 3x stakers leave and nobody new joins.' },
  { key: 'burstDay', group: 'Stress tests', label: 'Mass theft on day', unit: 'day', def: 0, min: 0, max: 3650, step: 1,
    tip: '0 = off. Many covered wallets robbed the same day (e.g. a wallet bug).' },
  { key: 'burstPct', group: 'Stress tests', label: 'Stakers hit in mass theft', unit: '%', def: 5, min: 0, max: 100, step: 1,
    tip: 'Share of stakers robbed on the mass-theft day.' },
  { key: 'freezeDay', group: 'Stress tests', label: 'Lending venue frozen from day', unit: 'day', def: 0, min: 0, max: 3650, step: 1,
    tip: '0 = off. Invested money can’t be taken out and earns nothing.' },
  { key: 'freezeDays', group: 'Stress tests', label: 'Freeze lasts', unit: 'days', def: 14, min: 1, max: 365, step: 1,
    tip: 'How long the invested money stays stuck.' },
];

export function assumptionDefaults() {
  return Object.fromEntries(ASSUMPTIONS.map((a) => [a.key, a.def]));
}

/** Problems with an assumptions object, as short sentences. Empty = valid. */
export function checkAssumptions(values) {
  const out = [];
  for (const a of ASSUMPTIONS) {
    const v = values[a.key];
    if (typeof v !== 'number' || Number.isNaN(v)) out.push(`${a.label}: missing`);
    else if (v < a.min || v > a.max) out.push(`${a.label}: must be ${a.min} to ${a.max} ${a.unit}`);
  }
  if (values.tierAPct + values.tierBPct > 100) out.push('Tier A + tier B wallets can’t be above 100%');
  return out;
}
