// One-click starting points for the page. Each only changes what it names;
// everything else stays at the contract defaults / base assumptions.
export const PRESETS = [
  { key: 'today', label: "Today's pool", tip: 'The new pool as it ships: $100K cap, default settings.',
    settings: {}, assumptions: {} },
  { key: 'partner1m', label: '$1M partner pool', tip: 'A partner runs a $1M pool with 10% backing.',
    settings: { poolCap: 1000000 }, assumptions: { backerPct: 10 } },
  { key: 'badYear', label: 'Bad year', tip: '10x normal thefts, a mass theft, a 60-day panic and a 2-week venue freeze.',
    settings: {}, assumptions: { theftPctYear: 3, burstDay: 200, burstPct: 5, panicDay: 210, freezeDay: 215, freezeDays: 14 } },
];
