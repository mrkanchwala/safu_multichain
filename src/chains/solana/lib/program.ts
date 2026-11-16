// The safu_pool program client. Every instruction is built from the IDL: its accounts (in IDL order,
// with the IDL's writable/signer flags), its PDA seeds, and its argument encoding. Marinade accounts
// come from the pool record and Marinade's own state (offsets and seeds are IDL constants).
import {
  AccountRole,
  address,
  appendTransactionMessageInstructions,
  compileTransaction,
  createSolanaRpc,
  createTransactionMessage,
  getAddressEncoder,
  getAddressDecoder,
  getBase64EncodedWireTransaction,
  getBase64Encoder,
  getProgramDerivedAddress,
  getSignatureFromTransaction,
  getTransactionDecoder,
  getTransactionEncoder,
  pipe,
  setTransactionMessageFeePayer,
  setTransactionMessageLifetimeUsingBlockhash,
} from "@solana/kit";
import type { Address, Instruction, Transaction } from "@solana/kit";
import { getSetComputeUnitLimitInstruction } from "@solana-program/compute-budget";
import { IDL, POOL, PROGRAM_ID, bytes, num } from "./pool";
import type { IdlAccountItem } from "./pool";
import { decodeAccount, encodeIx, programErrorName } from "./idl";

// The deployed site reads through its own /rpc (keys and failover stay on the server; the public
// devnet RPC limits each IP). Local dev, tests and localnet use rpcUrl directly.
const siteRpc = POOL.siteRpcPath && typeof window !== "undefined" && !import.meta.env.DEV ? new URL(POOL.siteRpcPath, window.location.origin).href : null;
export const rpc = createSolanaRpc(siteRpc ?? POOL.rpcUrl);

const SYSTEM_PROGRAM_ID = "11111111111111111111111111111111"; // hardcode-ok: Solana built-in program id, same on every cluster
const TOKEN_PROGRAM_ID = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"; // hardcode-ok: Solana built-in program id, same on every cluster
const TOKEN_2022_PROGRAM_ID = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"; // hardcode-ok: Solana built-in program id, same on every cluster
const addrEnc = getAddressEncoder();
const addrDec = getAddressDecoder();

export type Accounts = Record<string, string>;

async function pda(seeds: Uint8Array[], program: string = PROGRAM_ID): Promise<Address> {
  const [a] = await getProgramDerivedAddress({ programAddress: address(program), seeds });
  return a;
}

const key = (a: string) => new Uint8Array(addrEnc.encode(address(a)));

export const poolAddress = () => pda([bytes("SEED_POOL")]);
export const vaultAddress = async () => pda([bytes("SEED_VAULT"), key(await poolAddress())]);
export const stakeAddress = async (staker: string) => pda([bytes("SEED_STAKE"), key(await poolAddress()), key(staker)]);
export const backerAddress = async (backer: string) => pda([bytes("SEED_BACKER"), key(await poolAddress()), key(backer)]);

// --- reads ----------------------------------------------------------------------------------------

export type RawAccount = { data: Uint8Array; lamports: bigint; owner: string };

export async function fetchAccount(a: string): Promise<RawAccount | null> {
  const { value } = await rpc.getAccountInfo(address(a), { encoding: "base64", commitment: "confirmed" }).send();
  if (!value) return null;
  return {
    data: new Uint8Array(getBase64Encoder().encode(value.data[0])),
    lamports: BigInt(value.lamports),
    owner: value.owner,
  };
}

/** A program account decoded by its IDL layout; null if it doesn't exist. Refuses an account the
 *  program doesn't own (same check as the backend's _account_data). */
export async function readAccount<T>(name: string, a: string): Promise<T | null> {
  const acc = await fetchAccount(a);
  if (!acc) return null;
  if (acc.owner !== PROGRAM_ID) throw new Error(`${a} is not owned by the SAFU program`);
  return decodeAccount<T>(name, acc.data);
}

/** The chain's own clock (unix seconds), which can run behind the browser's. Every "ready at" check
 *  uses it, so a button never turns on before the program would accept the call. */
export async function chainNow(): Promise<number> {
  const slot = await rpc.getSlot({ commitment: "confirmed" }).send();
  const t = await rpc.getBlockTime(slot).send();
  return t === null ? Math.floor(Date.now() / 1000) : Number(t);
}

// --- Marinade leg ---------------------------------------------------------------------------------

export type PoolRecord = {
  admin: string; oracle: string; registry_writer: string;
  marinade_program: string; marinade_state: string; msol_mint: string; pool_msol: string;
  cluster: number; paused_until: bigint;
  /** Live adjustable settings, one slot per IDL `SETTING_*` constant: read them with `setting()`. */
  settings: bigint[];
  total_staked: bigint; total_stakers: bigint; total_backed: bigint; total_backed_pending: bigint;
  total_allocated: bigint; staker_yield_index: bigint; backer_yield_index: bigint;
};

const u64At = (b: Uint8Array, o: number) => new DataView(b.buffer, b.byteOffset).getBigUint64(o, true);
const u32At = (b: Uint8Array, o: number) => new DataView(b.buffer, b.byteOffset).getUint32(o, true);
const keyAt = (b: Uint8Array, o: number) => addrDec.decode(b.subarray(o, o + 32));

async function marinadeState(pool: PoolRecord): Promise<Uint8Array> {
  const st = await fetchAccount(pool.marinade_state);
  if (!st || st.owner !== pool.marinade_program) throw new Error("Marinade state not found");
  return st.data;
}

/** The `leg` account group every money-moving instruction takes. */
export async function legAccounts(pool: PoolRecord): Promise<Accounts> {
  const st = await marinadeState(pool);
  const m = (seed: string) => pda([key(pool.marinade_state), bytes(seed)], pool.marinade_program);
  const [solLeg, msolAuth, reserve, mintAuth] = await Promise.all([
    m("MARINADE_SEED_LIQ_POOL_SOL_LEG"),
    m("MARINADE_SEED_LIQ_POOL_MSOL_LEG_AUTHORITY"),
    m("MARINADE_SEED_RESERVE"),
    m("MARINADE_SEED_MSOL_MINT_AUTHORITY"),
  ]);
  return {
    marinade_program: pool.marinade_program,
    marinade_state: pool.marinade_state,
    msol_mint: pool.msol_mint,
    liq_pool_sol_leg: solLeg,
    liq_pool_msol_leg: keyAt(st, Number(num("MARINADE_STATE_LIQ_POOL_MSOL_LEG"))),
    liq_pool_msol_leg_authority: msolAuth,
    reserve,
    msol_mint_authority: mintAuth,
    treasury_msol: keyAt(st, Number(num("MARINADE_STATE_TREASURY_MSOL"))),
    pool_msol: pool.pool_msol,
  };
}

/** Marinade's instant-unstake fee right now, in bps, if `lamports` had to be unstaked
 *  (pool-core marinade::unstake_fee_bps). The payee pays it on the part the pool has to unstake. */
export async function unstakeFeeBps(pool: PoolRecord, lamports: bigint): Promise<number> {
  const st = await marinadeState(pool);
  const legs = await legAccounts(pool);
  const [solLeg, floor] = await Promise.all([
    fetchAccount(legs.liq_pool_sol_leg),
    rpc.getMinimumBalanceForRentExemption(0n).send(),
  ]);
  const available = (solLeg?.lamports ?? 0n) - BigInt(floor);
  const left = available > lamports ? available - lamports : 0n;
  const target = u64At(st, Number(num("MARINADE_STATE_LP_LIQUIDITY_TARGET")));
  const maxFee = u32At(st, Number(num("MARINADE_STATE_LP_MAX_FEE_BPS")));
  const minFee = u32At(st, Number(num("MARINADE_STATE_LP_MIN_FEE_BPS")));
  if (left >= target || target === 0n) return minFee;
  return maxFee - Number((BigInt(maxFee - minFee) * left) / target);
}

// --- instructions ---------------------------------------------------------------------------------

async function resolve(item: IdlAccountItem, given: Accounts, done: Accounts): Promise<string> {
  if (given[item.name]) return given[item.name];
  if (item.address) return item.address;
  if (item.pda) {
    const seeds: Uint8Array[] = [];
    for (const s of item.pda.seeds) {
      if (s.kind === "const") seeds.push(new Uint8Array(s.value));
      else if (s.kind === "account" && !s.path.includes(".") && (done[s.path] || given[s.path])) seeds.push(key(done[s.path] || given[s.path]));
      else throw new Error(`account ${item.name}: pass it in (seed ${s.path} can't be derived here)`);
    }
    return pda(seeds);
  }
  throw new Error(`account ${item.name} missing`);
}

/** One program instruction. `given` names accounts the IDL can't derive (signers, the claim, the
 *  Marinade leg); everything else comes from the IDL's fixed addresses and PDA seeds. */
export async function idlIx(name: string, given: Accounts, args: Record<string, unknown> = {}): Promise<Instruction> {
  const ix = IDL.instructions.find((i) => i.name === name);
  if (!ix) throw new Error(`IDL has no instruction ${name}`);
  const done: Accounts = {};
  const metas: { address: Address; role: AccountRole }[] = [];
  const walk = async (items: IdlAccountItem[]) => {
    for (const item of items) {
      if (item.accounts) {
        await walk(item.accounts);
        continue;
      }
      const a = await resolve(item, given, done);
      done[item.name] = a;
      const role = item.signer
        ? item.writable ? AccountRole.WRITABLE_SIGNER : AccountRole.READONLY_SIGNER
        : item.writable ? AccountRole.WRITABLE : AccountRole.READONLY;
      metas.push({ address: address(a), role });
    }
  };
  await walk(ix.accounts);
  return { programAddress: address(PROGRAM_ID), accounts: metas, data: encodeIx(name, args) };
}

// --- send -----------------------------------------------------------------------------------------

export type Signer = { address: string | null; solanaSignTransaction: (tx: Uint8Array) => Promise<Uint8Array> };

/** A failed simulation as `PROGRAM_ERROR:<IDL error name>` when this program refused it, or
 *  `MARINADE_REFUSED` when Marinade (called by the pool) did; friendly-error.ts turns either into a
 *  sentence. A custom error number only means something for the program that raised it: the first
 *  "Program <id> failed" line in the logs names that program (the innermost call fails first). */
function explain(e: unknown): Error {
  const seen = new Set<unknown>();
  const texts: string[] = [];
  const collect = (x: unknown) => {
    if (!x || typeof x !== "object" || seen.has(x)) return;
    seen.add(x);
    const o = x as { message?: string; context?: Record<string, unknown>; cause?: unknown };
    if (o.message) texts.push(o.message);
    for (const v of Object.values(o.context ?? {})) {
      if (Array.isArray(v)) texts.push(...v.filter((l): l is string => typeof l === "string"));
      else if (typeof v === "string") texts.push(v);
      else collect(v);
    }
    collect(o.cause);
  };
  collect(e);
  const failed = texts.map((t) => /Program (\w+) failed: custom program error: 0x([0-9a-f]+)/i.exec(t)).find(Boolean);
  if (failed && failed[1] !== PROGRAM_ID) {
    // System / Token program error 0x1 is "not enough funds" in the signer's wallet; it read as a
    // Marinade refusal before (founder's Solflare test, 2026-10-01). Only Marinade gets that message.
    if (failed[2] === "1" && [SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID].includes(failed[1])) {
      return new Error(`insufficient funds: ${failed[0]}`);
    }
    if (failed[1] === POOL.marinade.program) return new Error(`MARINADE_REFUSED: ${failed[0]}`);
    return new Error(`OTHER_PROGRAM_REFUSED: ${failed[0]}`);
  }
  for (const t of texts) {
    const m = /Program log: AnchorError .*Error Code: (\w+)\./.exec(t);
    if (m && (!failed || failed[1] === PROGRAM_ID) && IDL.errors?.some((x) => x.name === m[1])) return new Error(`PROGRAM_ERROR:${m[1]}`);
  }
  if (failed) {
    const name = programErrorName(parseInt(failed[2], 16));
    if (name) return new Error(`PROGRAM_ERROR:${name}`);
  }
  return e instanceof Error ? e : new Error(String(e));
}

/** Builds, has the wallet sign, sends and confirms. Returns the transaction signature. */
export async function send(wallet: Signer, ixs: Instruction[]): Promise<string> {
  if (!wallet.address) throw new Error("Connect a wallet first.");
  const { value: blockhash } = await rpc.getLatestBlockhash({ commitment: "confirmed" }).send();
  const msg = pipe(
    createTransactionMessage({ version: "legacy" }),
    (m) => setTransactionMessageFeePayer(address(wallet.address!), m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    (m) => appendTransactionMessageInstructions([getSetComputeUnitLimitInstruction({ units: POOL.computeUnitLimit }), ...ixs], m),
  );
  const signedBytes = await wallet.solanaSignTransaction(new Uint8Array(getTransactionEncoder().encode(compileTransaction(msg))));
  const signed = getTransactionDecoder().decode(signedBytes) as Transaction;
  const sig = getSignatureFromTransaction(signed as Parameters<typeof getSignatureFromTransaction>[0]);
  try {
    await rpc.sendTransaction(getBase64EncodedWireTransaction(signed), { encoding: "base64", preflightCommitment: "confirmed" }).send();
  } catch (e) {
    throw explain(e);
  }
  for (let i = 0; i < 60; i++) {
    const s = (await rpc.getSignatureStatuses([sig]).send()).value[0];
    if (s && (s.confirmationStatus === "confirmed" || s.confirmationStatus === "finalized")) {
      if (s.err) throw new Error("The transaction failed on-chain. Nothing moved. Try again.");
      return sig;
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error("The transaction wasn't confirmed in time. Check your wallet before trying again, so it doesn't happen twice.");
}
