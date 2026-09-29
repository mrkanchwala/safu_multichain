import {
  Address,
  BASE_FEE,
  Contract,
  TransactionBuilder,
  nativeToScVal,
  rpc,
  scValToNative,
  xdr,
} from "@stellar/stellar-sdk";
import { StellarWalletsKit } from "@creit.tech/stellar-wallets-kit";

import { NETWORK_PASSPHRASE, POOL_CONTRACT_ID, SOROBAN_RPC_URL } from "./network";

// Stage 6 wiring.
//
// Everything here is dormant until POOL_CONTRACT_ID in network.ts is filled in
// after deployment. `isLive()` is the single switch: while it returns false the
// panels keep running on the Stage 1 stubs, and no code path in this file is
// reachable. That keeps one build that works before and after the deploy rather
// than a branch that has to be merged under time pressure on the night.
//
// ONE contract now, not two -- the pivot's forked protection-pool replaces the
// old vault+backstop pair (2026-09-18).

const server = new rpc.Server(SOROBAN_RPC_URL);

export function isLive(): boolean {
  return POOL_CONTRACT_ID !== "";
}

export const poolId = () => POOL_CONTRACT_ID;

// --- argument helpers -------------------------------------------------------
// Types are explicit at every call site. `nativeToScVal` guesses otherwise, and
// a JS number silently becoming an i32 where the contract expects i128 is the
// kind of mismatch that only shows up as an opaque simulation failure.

export const addr = (value: string): xdr.ScVal => new Address(value).toScVal();
export const i128 = (value: bigint): xdr.ScVal => nativeToScVal(value, { type: "i128" });
export const u64 = (value: bigint | number): xdr.ScVal =>
  nativeToScVal(BigInt(value), { type: "u64" });
export const u32 = (value: number): xdr.ScVal => nativeToScVal(value, { type: "u32" });
export const bool = (value: boolean): xdr.ScVal => nativeToScVal(value, { type: "bool" });

/** BytesN<32> from a 64-char hex string -- what `submit_claim`'s tx_hash and
 * `claim_stream`'s claim_id both are. Added 2026-09-19 (A9) alongside the
 * pullClaimPayout fix, which is the first caller that needs a real one. */
export const bytesn32 = (hex: string): xdr.ScVal => {
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  if (!/^[0-9a-fA-F]{64}$/.test(clean)) {
    throw new Error(`bytesn32: expected 64 hex chars, got ${clean.length}: ${hex}`);
  }
  const bytes = new Uint8Array(32);
  for (let i = 0; i < 32; i++) bytes[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  return nativeToScVal(bytes, { type: "bytes" });
};

/** The inverse of `bytesn32` -- `scValToNative` returns a raw byte array for
 * a BytesN field (`active_claim_id`, `reserved_claim_id`), not a hex string. */
export const hexFromBytes = (bytes: Uint8Array): string =>
  Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");

/**
 * Reads a contract without signing or submitting anything.
 *
 * A simulation runs the real contract against real ledger state, so a read here
 * returns exactly what a transaction would have computed. It costs nothing and
 * needs no wallet, which is why every panel read goes through it rather than
 * mirroring contract logic in TypeScript.
 */
export async function readContract<T = unknown>(
  contractId: string,
  method: string,
  args: xdr.ScVal[] = [],
  sourceAccount?: string,
): Promise<T> {
  // A read still needs a source account to build against. Any funded account
  // works because nothing is submitted; this falls back to the all-zero account
  // so reads work before a wallet is connected.
  const source =
    sourceAccount ?? "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF";
  const account = new (await import("@stellar/stellar-sdk")).Account(source, "0");

  const tx = new TransactionBuilder(account, {
    fee: BASE_FEE,
    networkPassphrase: NETWORK_PASSPHRASE,
  })
    .addOperation(new Contract(contractId).call(method, ...args))
    .setTimeout(30)
    .build();

  const sim = await server.simulateTransaction(tx);
  if (rpc.Api.isSimulationError(sim)) {
    throw new Error(`${method}: ${sim.error}`);
  }
  if (!sim.result?.retval) {
    throw new Error(`${method}: simulation returned no value`);
  }
  return scValToNative(sim.result.retval) as T;
}

/**
 * Builds, simulates, signs and submits a state-changing call, then waits for
 * the ledger to confirm it.
 *
 * Simulating before signing matters: it is what discovers the footprint and
 * resource fees, and it surfaces a contract error as a readable message rather
 * than as a failed transaction the user has already paid for and signed.
 */
export async function invokeContract(
  contractId: string,
  method: string,
  args: xdr.ScVal[],
  signerAddress: string,
): Promise<string> {
  const account = await server.getAccount(signerAddress);

  const built = new TransactionBuilder(account, {
    fee: BASE_FEE,
    networkPassphrase: NETWORK_PASSPHRASE,
  })
    .addOperation(new Contract(contractId).call(method, ...args))
    .setTimeout(60)
    .build();

  const sim = await server.simulateTransaction(built);
  if (rpc.Api.isSimulationError(sim)) {
    throw new Error(`${method}: ${sim.error}`);
  }

  const prepared = rpc.assembleTransaction(built, sim).build();

  const { signedTxXdr } = await StellarWalletsKit.signTransaction(prepared.toXDR(), {
    networkPassphrase: NETWORK_PASSPHRASE,
    address: signerAddress,
  });

  const sent = await server.sendTransaction(
    TransactionBuilder.fromXDR(signedTxXdr, NETWORK_PASSPHRASE),
  );
  if (sent.status === "ERROR") {
    throw new Error(`${method}: submission rejected`);
  }

  return waitForTransaction(sent.hash, method);
}

/**
 * Polls until the ledger closes on the transaction.
 *
 * Testnet confirmation is often just slow rather than broken, so this waits
 * rather than treating the first NOT_FOUND as a failure. That distinction has
 * cost real debugging time before.
 */
async function waitForTransaction(hash: string, method: string): Promise<string> {
  const deadline = Date.now() + 45_000;
  while (Date.now() < deadline) {
    const result = await server.getTransaction(hash);
    if (result.status === rpc.Api.GetTransactionStatus.SUCCESS) {
      return hash;
    }
    if (result.status === rpc.Api.GetTransactionStatus.FAILED) {
      throw new Error(`${method}: rejected on chain`);
    }
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error(`${method}: still unconfirmed after 45s (hash ${hash})`);
}
