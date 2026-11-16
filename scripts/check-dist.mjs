// After every build (npm postbuild), from hq/site.toml [guards]:
//   never_ship: fail if dist/ contains any of these strings, e.g. the hidden USDC pool's program id (B42 review A3);
//   max_entry_kb / max_chunk_kb: fail if the entry script or any other JS file grows past its budget (CIE P-2).
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

// The entry is the script index.html loads; every other .js file is a chunk loaded on demand.
const entry = /src="\/assets\/([^"]+\.js)"/.exec(readFileSync(join(root, "dist/index.html"), "utf8"))?.[1];
if (!entry) {
  console.error("dist check: no entry script found in dist/index.html");
  process.exit(1);
}
const over = files(join(root, "dist/assets"))
  .filter((f) => f.endsWith(".js"))
  .map((f) => ({ f, kb: statSync(f).size / 1024, max: f.endsWith(`/${entry}`) ? guards.max_entry_kb : guards.max_chunk_kb }))
  .filter(({ kb, max }) => kb > max);
if (over.length) {
  console.error(`dist/ JS over budget (hq/site.toml [guards]):\n${over.map(({ f, kb, max }) => `${f.slice(root.length)}: ${kb.toFixed(0)} KB > ${max} KB`).join("\n")}`);
  process.exit(1);
}
console.log(`dist check: none of ${banned.length} never-ship strings found; entry and chunks within budget`);
