import { describe, expect, it } from "vitest";
import { coverRows, coverStatus, shortAddr } from "./StakeSteps";

const row = (k: string) => ({ key: k, label: "Solana", addr: k });

describe("coverStatus", () => {
  it("never shows green without a stake or without the pool's own count", () => {
    expect(coverStatus(false, 2, 3)).toEqual({ text: "2/3", cls: "" });
    expect(coverStatus(true, null, 3)).toEqual({ text: "Checking…", cls: "" });
  });
  it("is amber at 0 and green from 1", () => {
    expect(coverStatus(true, 0, 2)).toEqual({ text: "0/2 · Not protected yet", cls: "warn" });
    expect(coverStatus(true, 1, 2)).toEqual({ text: "1/2 · Protected", cls: "ok" });
  });
});

describe("coverRows", () => {
  it("cuts a stale browser list to the pool's count", () => {
    expect(coverRows([row("a"), row("b")], 1).rows.map((r) => r.key)).toEqual(["a"]);
  });
  it("adds unnamed rows for wallets this browser can't name", () => {
    const { rows, unnamed } = coverRows([row("a")], 3);
    expect(unnamed).toBe(2);
    expect(rows.map((r) => r.label)).toEqual(["Solana", "Wallet 2", "Wallet 3"]);
  });
  it("keeps the browser list while the count is unknown", () => {
    expect(coverRows([row("a")], null)).toEqual({ rows: [row("a")], unnamed: 0 });
  });
});

it("shortAddr keeps first and last 4", () => {
  expect(shortAddr("7ggVh5ENabcdefgh2czd")).toBe("7ggV…2czd");
  expect(shortAddr("short")).toBe("short");
});
