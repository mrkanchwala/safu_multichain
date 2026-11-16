import { describe, expect, it } from "vitest";
import { SITE } from "../shell/lib/site";
import { ADAPTER_IDS, chainFromSearch } from "./registry";

describe("chain registry", () => {
  it("has an adapter for every chain hq/site.toml enables", () => {
    for (const id of SITE.chains.enabled) expect(ADAPTER_IDS).toContain(id);
  });

  it("defaults to the HQ default chain", () => {
    expect(SITE.chains.enabled).toContain(SITE.chains.default);
    expect(chainFromSearch("")).toBe(SITE.chains.default);
  });

  it("takes ?chain= only for an enabled chain", () => {
    expect(chainFromSearch("?chain=solana", ["stellar", "solana"], "stellar")).toBe("solana");
    expect(chainFromSearch("?chain=solana", ["stellar"], "stellar")).toBe("stellar");
    expect(chainFromSearch("?chain=", ["stellar"], "stellar")).toBe("stellar");
    expect(chainFromSearch("?chain=nope&x=1", ["stellar"], "stellar")).toBe("stellar");
  });
});
