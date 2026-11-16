import { describe, expect, it } from "vitest";
import { Keypair } from "@stellar/stellar-sdk";
import { addressChain, walletChainMismatch } from "./wallet-chain";
import { CHAINS, EVM_CHAIN_KEY } from "./network";

const SOL = "7R8GYzEM7tGcKCVgc6Wdi5yQv4TuB1mFke7fh9fbFYjf";
const EVM = "0xDeadBeef0000000000000000000000000000dEaD";
const XLM = Keypair.random().publicKey();
const label = (id: string) => CHAINS.find((c) => c.id === id)!.label;

describe("walletChainMismatch", () => {
  it("passes an address on its own chain", () => {
    expect(walletChainMismatch("solana", SOL)).toBeNull();
    expect(walletChainMismatch(EVM_CHAIN_KEY, EVM)).toBeNull();
    expect(walletChainMismatch("stellar", XLM)).toBeNull();
    expect(walletChainMismatch("solana", `  ${SOL} `)).toBeNull();
  });

  it("names Solana for a Solana address with Ethereum selected (the 29 Oct case)", () => {
    expect(walletChainMismatch(EVM_CHAIN_KEY, SOL)).toBe(`That's a ${label("solana")} address, so pick ${label("solana")} above.`);
  });

  it("names the right chain for every other mix-up", () => {
    expect(walletChainMismatch("solana", EVM)).toContain(label(EVM_CHAIN_KEY));
    expect(walletChainMismatch("stellar", SOL)).toContain(label("solana"));
    expect(walletChainMismatch(EVM_CHAIN_KEY, XLM)).toContain(label("stellar"));
  });

  it("refuses garbage", () => {
    for (const w of ["0x123", "SoLWallet", "GNOTAKEY", "", "0OIl".repeat(11)]) {
      expect(addressChain(w)).toBeNull();
      expect(walletChainMismatch("solana", w)).toMatch(/^That isn't a valid/);
    }
  });
});
