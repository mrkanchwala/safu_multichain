import { describe, expect, it } from "vitest";

import { toFriendlyError } from "./friendly-error";

describe("toFriendlyError", () => {
  it("says 'not enough SOL' for a System program 0x1, not a Marinade refusal", () => {
    const r = toFriendlyError(new Error("insufficient funds: Program 11111111111111111111111111111111 failed: custom program error: 0x1"));
    expect(r.message).toBe("Not enough SOL in this wallet.");
  });

  it("matches a known backend rejection reason", () => {
    const result = toFriendlyError(new Error("WALLET_NOT_COVERED: wallet not registered"));
    expect(result.message).toContain("isn't one of your covered wallets");
    expect(result.raw).toContain("WALLET_NOT_COVERED");
  });

  it("explains the staking-wallet refusal", () => {
    const r = toFriendlyError(new Error("STAKER_WALLET: a covered wallet can't be the same as your staking wallet."));
    expect(r.message).toContain("separate wallet");
  });

  it("matches a wallet-rejection message case-insensitively", () => {
    expect(toFriendlyError(new Error("User Rejected the request")).message).toBe("Request was declined in the wallet.");
  });

  it("falls back to a generic message for an unmatched reason, never leaking the raw string", () => {
    const result = toFriendlyError(new Error("SOME_UNMAPPED_CODE"));
    expect(result.message).toContain("didn't go through");
    expect(result.message).not.toContain("SOME_UNMAPPED_CODE");
  });

  it("treats a wallet's closed-window rejection (a plain object) as a cancel", () => {
    expect(toFriendlyError({ code: -1, message: "The user closed the modal." }).cancelled).toBe(true);
  });

  it("explains a covered wallet that fails the history floors, with its numbers", () => {
    const tx = toFriendlyError(new Error("WALLET_INELIGIBLE: covered wallet had 8 transactions at the hack; at least 10 are required"));
    expect(tx.message).toContain("only 8 transactions");
    expect(tx.message).toContain("at least 10");
    const age = toFriendlyError(new Error("WALLET_INELIGIBLE: covered wallet was 3d old at the hack; at least 60 days are required"));
    expect(age.message).toContain("only 3 days old");
    expect(toFriendlyError(new Error("WALLET_INELIGIBLE: something new")).message).toContain("enough history");
  });

  it("blames our side, not the connection, when the API answers 5xx", () => {
    expect(toFriendlyError(new Error("Request failed (500)")).message).toContain("on our side");
    expect(toFriendlyError(new Error("Failed to fetch")).message).toContain("Check your connection");
  });

  it("shows the app's own plain messages as written", () => {
    expect(toFriendlyError(new Error("Connect a wallet first.")).message).toBe("Connect a wallet first.");
  });

  it("translates a program error by its IDL name", () => {
    expect(toFriendlyError(new Error("PROGRAM_ERROR:NothingVested")).message).toContain("Nothing new to collect");
    expect(toFriendlyError(new Error("PROGRAM_ERROR:StakeOutOfRange")).message).toContain("stake limits");
  });

  it("never shows an unmapped program error name", () => {
    const r = toFriendlyError(new Error("PROGRAM_ERROR:NotOracle"));
    expect(r.message).not.toMatch(/NotOracle|PROGRAM/);
  });

  it("explains a Marinade refusal without naming a pool rule", () => {
    const r = toFriendlyError(new Error("MARINADE_REFUSED: Program MarBmsSgKXdrN1egZf5sqe1TMai9K1rChYNDJgjq7aD failed: custom program error: 0x178e"));
    expect(r.message).toContain("Marinade");
    expect(r.message).not.toMatch(/0x|MARINADE_/);
  });

  it("explains missing SOL", () => {
    expect(toFriendlyError(new Error("Attempt to debit an account but found no record of a prior credit.")).message).toContain("SOL");
  });

  it("shows no reason code in any backend message", () => {
    for (const code of ["CHAIN_UNAVAILABLE: x", "BROADCAST_FAILED: x", "PRICE_UNAVAILABLE: x", "TooManyWallets: x"]) {
      expect(toFriendlyError(new Error(code)).message).not.toMatch(/[A-Z]{3,}_[A-Z]|TooMany/);
    }
  });
});
