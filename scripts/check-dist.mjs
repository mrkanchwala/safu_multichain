// After every build (npm postbuild): fail if dist/ contains anything hq/site.toml lists under
// [guards] never_ship, e.g. the hidden Solana USDC pool's program id (B42 review A3).
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { parse } from "smol-toml";

const root = new URL("..", import.meta.url).pathname;
const { guards } = parse(readFileSync(join(root, "hq/site.toml"), "utf8"));
const banned = guards?.never_ship ?? [];

const files = (dir) =>
  readdirSync(dir).flatMap((n) => (statSync(join(dir, n)).isDirectory() ? files(join(dir, n)) : [join(dir, n)]));

const hits = files(join(root, "dist")).flatMap((f) => {
  const text = readFileSync(f, "latin1");
  return banned.filter((s) => text.includes(s)).map((s) => `${f.slice(root.length)}: ${s}`);
});
if (hits.length) {
  console.error(`dist/ contains never-ship strings (hq/site.toml [guards]):\n${hits.join("\n")}`);
  process.exit(1);
}
console.log(`dist check: none of ${banned.length} never-ship strings found`);
