import { describe, expect, it } from "vitest";
import { fmtExact, fmtYield } from "./reads";

describe("fmtYield", () => {
  it("keeps sub-cent yield visible", () => {
    expect(fmtYield(10721n)).toBe("0.00107"); // 0.0010721 USDC, the 20 Oct mainnet case
    expect(fmtYield(1n)).toBe("0.0000001");
  });
  it("uses two decimals from a cent up, and for zero", () => {
    expect(fmtYield(0n)).toBe("0.00");
    expect(fmtYield(1_234_567n)).toBe("0.12");
  });
});

describe("fmtExact", () => {
  it("shows the yield part of a balance", () => {
    expect(fmtExact(10_010_721n)).toBe("1.0010721");
  });
  it("never shows fewer than two decimals", () => {
    expect(fmtExact(10_000_000n)).toBe("1.00");
    expect(fmtExact(15_000_000n)).toBe("1.50");
  });
});
