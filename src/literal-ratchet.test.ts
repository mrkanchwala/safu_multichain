// Literal ratchet (B42, 2026-11-16): values belong in hq/site.toml, config/pool.<network>.json or the
// contract, never typed into source. This counts the ones still typed in, per file, and fails when a
// count goes UP (a new hard-wired value) or DOWN without lowering the baseline (lock in the win).
// After removing literals: RATCHET_WRITE=1 npx vitest run src/literal-ratchet.test.ts
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { env } from "node:process"; // the browser polyfill replaces the global `process`
import { describe, expect, it } from "vitest";

const SRC = new URL(".", import.meta.url).pathname;
const BASELINE = join(SRC, "literal-ratchet.baseline.json");

const PATTERNS: Record<string, RegExp> = {
  url: /https?:\/\/[^\s"'`)}]+/g,
  stellarAddress: /\b[GC][A-Z2-7]{55}\b/g,
  evmAddress: /\b0x[0-9a-fA-F]{40}\b/g,
  solanaAddress: /["'`][1-9A-HJ-NP-Za-km-z]{32,44}["'`]/g,
  ruleMultiple: /\b[1-9]\d*(\.\d+)?x\b/g, // 15x, not the 0x hex prefix
  ruleDays: /\b\d+[- ]days?\b/g,
};

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

/** Comments may cite docs and addresses; only code and copy count. `//` after `:` is a URL, not a comment. */
function stripComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
}

export function countLiterals(text: string): number {
  const code = stripComments(text);
  return Object.values(PATTERNS).reduce((n, re) => n + (code.match(re)?.length ?? 0), 0);
}

function currentCounts(): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const path of sourceFiles(SRC).sort()) {
    const n = countLiterals(readFileSync(path, "utf8"));
    if (n > 0) counts[relative(SRC, path)] = n;
  }
  return counts;
}

describe("literal ratchet", () => {
  it("counts each kind of literal", () => {
    expect(countLiterals('const u = "https://example.com/x";')).toBe(1);
    expect(countLiterals("up to 15x your stake, after a 90-day wait")).toBe(2);
    expect(countLiterals(`"0x${"a".repeat(40)}"`)).toBe(1);
    expect(countLiterals(`const c = "C${"A".repeat(55)}"`)).toBe(1);
    expect(countLiterals("const ok = SITE.site.url;")).toBe(0);
    expect(countLiterals("type Hex = `0x${string}`;")).toBe(0);
    expect(countLiterals("// see https://docs.example.com\n/* 15x */ const a = 1;")).toBe(0);
  });

  it("never gains a hard-wired value, and locks in every removal", () => {
    const counts = currentCounts();
    if (env.RATCHET_WRITE) writeFileSync(BASELINE, JSON.stringify(counts, null, 2) + "\n");
    const baseline = JSON.parse(readFileSync(BASELINE, "utf8")) as Record<string, number>;
    expect(counts).toEqual(baseline);
  });
});
