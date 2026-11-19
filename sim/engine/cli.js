// Prints the headline numbers for each preset: node sim/engine/cli.js [runs]
import { defaults } from './settings.js';
import { assumptionDefaults } from './assumptions.js';
import { PRESETS } from './presets.js';
import { runMany } from './model.js';

const runs = Number(process.argv[2] || 50);
const money = (x) => `$${(Math.round(x) || 0).toLocaleString('en-US')}`;
for (const p of PRESETS) {
  const s = { ...defaults(), ...p.settings };
  const a = { ...assumptionDefaults(), ...p.assumptions, runs };
  const r = runMany(s, a);
  console.log(`\n${p.label}`);
  console.log(`  paid on time, every claim and exit: ${r.onTimePct.toFixed(0)}% of ${r.runs} runs`);
  console.log(`  money held vs owed, worst moment: ${(r.worstRatio * 100).toFixed(0)}% (typical run ${(r.typicalRatio * 100).toFixed(0)}%)`);
  console.log(`  thefts per run: ${r.thefts} | paid out: ${money(r.paidOut)} | longest wait in line: ${r.worstWaitInLine} days`);
  console.log(`  theft to fully paid: ${r.daysToPaid} days typical, ${r.daysToPaidSlow} slowest 5%`);
  console.log(`  first cash shortfall: ${Number.isFinite(r.firstShortDay) ? `day ${r.firstShortDay}` : 'never'}`);
  console.log(`  yield a year: stakers ${money(r.stakerYieldPerYear)}, backers ${money(r.backerYieldPerYear)}, SAFU ${money(r.safuPerYear)}`);
}
