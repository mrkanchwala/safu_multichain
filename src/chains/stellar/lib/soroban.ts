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

import { NETWORK_PASSPHRASE, POOL_CONTRACT_ID, SOROBAN_RPC_URLS } from "./network";

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

// One server per RPC in the pool file, primary first (CIE A-1, B42). `onRpc` moves to the next only when one
// can't be reached (no answer, HTTP 429 or 5xx); an answer, including a contract error, is final. A resent
// signed transaction keeps its hash, so a send retried on another RPC can't land twice.
const servers = SOROBAN_RPC_URLS.map((url) => new rpc.Server(url));

function unreachable(e: unknown): boolean {
  const err = e as { response?: { status?: number }; isAxiosError?: boolean } | null;
  const status = err?.response?.status;
  if (status !== undefined) return status === 429 || status >= 500;
  return err?.isAxiosError === true || e instanceof TypeError; // no answer at all: network, DNS, CORS
}

async function onRpc<T>(call: (server: rpc.Server) => Promise<T>): Promise<T> {
  let last: unknown;
  for (const server of servers) {
    try {
      return await call(server);
    } catch (e) {
      if (!unreachable(e)) throw e;
      last = e;
    }
  }
  throw last;
}

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
/** One value from a contract's instance storage, by its DataKey name (e.g. "PoolCap"); undefined if unset.
 *  For numbers the contract stores but has no getter for. */
export async function readInstanceValue<T = unknown>(contractId: string, name: string): Promise<T | undefined> {
  const entry = await onRpc((s) =>
    s.getContractData(contractId, xdr.ScVal.scvLedgerKeyContractInstance(), rpc.Durability.Persistent),
  );
  const storage = (entry.val as unknown as { contractData: { val: { instance: { storage?: { key: xdr.ScVal; val: xdr.ScVal }[] } } } })
    .contractData.val.instance.storage ?? [];
  const hit = storage.find((e) => JSON.stringify(scValToNative(e.key)) === JSON.stringify([name]));
  return hit ? (scValToNative(hit.val) as T) : undefined;
}

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

  const sim = await onRpc((s) => s.simulateTransaction(tx));
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
  const account = await onRpc((s) => s.getAccount(signerAddress));

  const built = new TransactionBuilder(account, {
    fee: BASE_FEE,
    networkPassphrase: NETWORK_PASSPHRASE,
  })
    .addOperation(new Contract(contractId).call(method, ...args))
    .setTimeout(60)
    .build();

  const sim = await onRpc((s) => s.simulateTransaction(built));
  if (rpc.Api.isSimulationError(sim)) {
    throw new Error(`${method}: ${sim.error}`);
  }

  const prepared = rpc.assembleTransaction(built, sim).build();

  const { signedTxXdr } = await StellarWalletsKit.signTransaction(prepared.toXDR(), {
    networkPassphrase: NETWORK_PASSPHRASE,
    address: signerAddress,
  });

  const signed = TransactionBuilder.fromXDR(signedTxXdr, NETWORK_PASSPHRASE);
  const sent = await onRpc((s) => s.sendTransaction(signed));
  if (sent.status === "ERROR") {
    throw new Error(`${method}: submission rejected (${resultCode(sent.errorResult)})`);
  }
  if (sent.status === "TRY_AGAIN_LATER") {
    throw new Error(`${method}: submission rejected (tryAgainLater)`);
  }

  return waitForTransaction(sent.hash, method);
}

/**
 * Stellar's own reason for refusing or failing a transaction, e.g. "txTooLate" or
 * "txFailed/invokeHostFunctionTrapped". Kept in the error so friendly-error.ts can
 * say what actually happened instead of a generic line (added 2026-10-13 after a
 * staker hit two unexplained refusals that left no trace anywhere).
 */
function resultCode(result?: xdr.TransactionResult): string {
  const r = result?.result;
  if (!r) return "unknown";
  if (r.type !== "txFailed") return r.type;
  const op = r.results[0];
  if (op?.type === "opInner" && op.tr.type === "invokeHostFunction") {
    return `txFailed/${op.tr.invokeHostFunctionResult.type}`;
  }
  return `txFailed/${op?.type ?? "unknown"}`;
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
    const result = await onRpc((s) => s.getTransaction(hash));
    if (result.status === rpc.Api.GetTransactionStatus.SUCCESS) {
      return hash;
    }
    if (result.status === rpc.Api.GetTransactionStatus.FAILED) {
      throw new Error(`${method}: rejected on chain (${resultCode(result.resultXdr)})`);
    }
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error(`${method}: still unconfirmed after 45s (hash ${hash})`);
}
