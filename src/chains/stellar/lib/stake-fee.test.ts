import { describe, expect, it } from "vitest";
import { burnForStake } from "./crosschain";

// What lands on Stellar for a burn, with Circle's 1 bp fee rounded either way.
const lands = (b: bigint) => [b - (b + 9_999n) / 10_000n, b - b / 10_000n];
const MIN = 1_000_000n;
const MAX = 10_000_000n;

describe("burnForStake", () => {
  it("sends the fee on top so 1 USDC lands at least 1 USDC (the 29 Oct 0.9999 case)", () => {
    const b = burnForStake(MIN, MIN, MAX);
    expect(b).toBe(1_000_101n);
    expect(lands(b)[0]).toBeGreaterThanOrEqual(MIN);
  });

  it("never lands above the maximum stake", () => {
    const b = burnForStake(MAX, MIN, MAX);
    expect(lands(b)[1]).toBeLessThanOrEqual(MAX);
    expect(lands(b)[0]).toBeGreaterThanOrEqual(MIN);
  });

  it("lands within one unit of the typed amount across the range", () => {
    for (let s = MIN; s <= MAX; s += 123_457n) {
      const [lo, hi] = lands(burnForStake(s, MIN, MAX));
      expect(lo).toBeGreaterThanOrEqual(s);
      expect(hi - s).toBeLessThanOrEqual(1n);
    }
  });
});
