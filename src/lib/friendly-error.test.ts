import { describe, expect, it } from "vitest";

import { toFriendlyError } from "./friendly-error";

describe("toFriendlyError", () => {
  it("matches a known backend rejection reason", () => {
    const result = toFriendlyError(new Error("WALLET_NOT_COVERED: wallet not registered"));
    expect(result.message).toBe(
      "That wallet isn't one of your covered wallets, or wasn't added before this happened. Covered wallets have to be added before anything happens to them.",
    );
    expect(result.raw).toContain("WALLET_NOT_COVERED");
  });

  it("matches a wallet-rejection message case-insensitively", () => {
    const result = toFriendlyError(new Error("User Rejected the request"));
    expect(result.message).toBe("Request was declined in the wallet.");
  });

  it("matches an insufficient-XLM message", () => {
    const result = toFriendlyError(new Error("insufficient XLM balance for fees"));
    expect(result.message).toBe("Not enough XLM in the wallet to cover the network fee.");
  });

  it("falls back to a generic message for an unmatched reason, never leaking the raw string", () => {
    const result = toFriendlyError(new Error("SOME_UNMAPPED_CONTRACT_ERROR_CODE"));
    expect(result.message).toContain("didn't go through");
    expect(result.message).not.toContain("SOME_UNMAPPED_CONTRACT_ERROR_CODE");
    expect(result.raw).toContain("SOME_UNMAPPED_CONTRACT_ERROR_CODE");
  });

  it("treats the Stellar kit's closed-modal rejection (a plain object) as a cancel", () => {
    const result = toFriendlyError({ code: -1, message: "The user closed the modal." });
    expect(result.cancelled).toBe(true);
    expect(result.message).not.toContain("didn't go through");
  });

  it("reads the message of a plain-object rejection", () => {
    const result = toFriendlyError({ code: 4001, message: "User rejected the request." });
    expect(result.message).toBe("Request was declined in the wallet.");
  });

  it("explains WalletConnect's unsupported-chains rejection", () => {
    const result = toFriendlyError({ code: 5100, message: "Unsupported chains." });
    expect(result.message).toContain("test network");
  });

  it("shows the app's own plain messages as written", () => {
    expect(toFriendlyError(new Error("Connect a wallet first.")).message).toBe("Connect a wallet first.");
    expect(toFriendlyError(new Error("Wrong network -- switch your wallet to Sepolia.")).message).toContain("Sepolia");
  });

  it("stringifies a non-Error thrown value instead of throwing", () => {
    const result = toFriendlyError("plain string rejection");
    expect(result.raw).toBe("plain string rejection");
  });

  // 2026-09-25 founder live test: these all reached users as the generic line.
  it("translates a contract error inside a relay detail", () => {
    const r = toFriendlyError(new Error("PREPARE_FAILED: HostError: Error(Contract, #62) ..."));
    expect(r.message).toContain("payout hasn't started");
  });

  it("translates a contract error from a direct Stellar call", () => {
    expect(toFriendlyError(new Error("claim_stream: HostError: Error(Contract, #63)")).message).toContain("Nothing new to collect");
  });

  it("never shows an unmapped contract code", () => {
    const r = toFriendlyError(new Error("SIMULATION_FAILED: Error(Contract, #999)"));
    expect(r.message).not.toMatch(/#999|SIMULATION|Contract/);
  });

  it("explains a Solana wallet that signs in the wrong format", () => {
    expect(toFriendlyError(new Error("WALLET_SIGNATURE_FORMAT")).message).toContain("Phantom");
  });

  it("explains a rejected signature", () => {
    expect(toFriendlyError(new Error("PREPARE_FAILED: Error(Auth, InvalidAction)")).message).toContain("signature");
  });

  it("explains missing gas on each chain", () => {
    expect(toFriendlyError(new Error("insufficient funds for gas * price + value")).message).toContain("Not enough ETH");
    expect(toFriendlyError(new Error("Attempt to debit an account but found no record of a prior credit.")).message).toContain("SOL");
  });

  it("tells a stuck deposit its money is safe", () => {
    expect(toFriendlyError(new Error("DEPOSIT_FAILED: attestation pending")).message).toContain("safe");
  });

  it("shows no reason code in any backend message", () => {
    for (const code of ["RATE_LIMITED: x", "RELAY_BUSY: x", "TX_FAILED: abc", "NOT_CONFIRMED: abc", "REFUSED: x", "UNKNOWN_OR_EXPIRED: prepare again"]) {
      expect(toFriendlyError(new Error(code)).message).not.toMatch(/[A-Z]{3,}_[A-Z]/);
    }
  });
});

