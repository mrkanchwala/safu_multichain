// Borsh encode/decode driven by the program's IDL (same idea as backend/solana_pool.py decode_account):
// account layouts, instruction args and discriminators are read from the IDL, never written out here.
import { getAddressDecoder, getAddressEncoder } from "@solana/kit";
import type { Address } from "@solana/kit";
import { IDL } from "./pool";
import type { IdlField, IdlType } from "./pool";

const addrDec = getAddressDecoder();
const addrEnc = getAddressEncoder();

const INT: Record<string, { size: number; signed: boolean }> = {
  u8: { size: 1, signed: false }, i8: { size: 1, signed: true },
  u16: { size: 2, signed: false }, i16: { size: 2, signed: true },
  u32: { size: 4, signed: false }, i32: { size: 4, signed: true },
  u64: { size: 8, signed: false }, i64: { size: 8, signed: true },
  u128: { size: 16, signed: false }, i128: { size: 16, signed: true },
};

function typeDef(name: string) {
  const t = IDL.types.find((x) => x.name === name);
  if (!t) throw new Error(`IDL has no type ${name}`);
  return t.type;
}

/** Integers up to 32 bits decode to number, wider ones to bigint. Pubkeys decode to base58 strings,
 *  unit enums to their variant name, structs to plain objects keyed by field name. */
function read(type: IdlType, b: Uint8Array, o: { at: number }): unknown {
  if (typeof type === "string") {
    if (type === "bool") return b[o.at++] !== 0;
    if (type === "pubkey") {
      const v = addrDec.decode(b.subarray(o.at, o.at + 32));
      o.at += 32;
      return v;
    }
    const int = INT[type];
    if (!int) throw new Error(`unsupported IDL type ${type}`);
    let v = 0n;
    for (let i = int.size - 1; i >= 0; i--) v = (v << 8n) | BigInt(b[o.at + i]);
    o.at += int.size;
    if (int.signed && v >= 1n << BigInt(int.size * 8 - 1)) v -= 1n << BigInt(int.size * 8);
    return int.size <= 4 ? Number(v) : v;
  }
  if ("array" in type) {
    const [inner, len] = type.array;
    if (inner === "u8") {
      const v = b.slice(o.at, o.at + len);
      o.at += len;
      return v;
    }
    return Array.from({ length: len }, () => read(inner, b, o));
  }
  if ("option" in type) return b[o.at++] === 0 ? null : read(type.option, b, o);
  if ("vec" in type) {
    const len = read("u32", b, o) as number;
    return Array.from({ length: len }, () => read(type.vec, b, o));
  }
  const def = typeDef(type.defined.name);
  if (def.kind === "enum") return def.variants[b[o.at++]].name;
  return readFields(def.fields, b, o);
}

function readFields(fields: IdlField[], b: Uint8Array, o: { at: number }): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const f of fields) out[f.name] = read(f.type, b, o);
  return out;
}

function write(type: IdlType, v: unknown, out: number[]): void {
  if (typeof type === "string") {
    if (type === "bool") return void out.push(v ? 1 : 0);
    if (type === "pubkey") return void out.push(...addrEnc.encode(v as Address));
    const int = INT[type];
    if (!int) throw new Error(`unsupported IDL type ${type}`);
    let x = BigInt(v as bigint | number);
    if (x < 0n) x += 1n << BigInt(int.size * 8);
    for (let i = 0; i < int.size; i++, x >>= 8n) out.push(Number(x & 0xffn));
    return;
  }
  if ("array" in type) {
    const arr = Array.from(v as ArrayLike<unknown>);
    if (arr.length !== type.array[1]) throw new Error(`expected ${type.array[1]} items, got ${arr.length}`);
    return arr.forEach((x) => write(type.array[0], x, out));
  }
  if ("option" in type) {
    if (v === null || v === undefined) return void out.push(0);
    out.push(1);
    return write(type.option, v, out);
  }
  if ("vec" in type) {
    const arr = Array.from(v as ArrayLike<unknown>);
    write("u32", arr.length, out);
    return arr.forEach((x) => write(type.vec, x, out));
  }
  const def = typeDef(type.defined.name);
  if (def.kind === "enum") return void out.push(def.variants.findIndex((x) => x.name === v));
  for (const f of def.fields) write(f.type, (v as Record<string, unknown>)[f.name], out);
}

const eq = (a: ArrayLike<number>, b: ArrayLike<number>) => a.length === b.length && Array.from(a).every((x, i) => x === b[i]);

/** An account's fields, after checking its 8-byte discriminator. */
export function decodeAccount<T = Record<string, unknown>>(name: string, data: Uint8Array): T {
  const acc = IDL.accounts.find((a) => a.name === name);
  if (!acc) throw new Error(`IDL has no account ${name}`);
  if (!eq(data.subarray(0, 8), acc.discriminator)) throw new Error(`not a ${name} account`);
  const def = typeDef(name);
  if (def.kind !== "struct") throw new Error(`${name} is not a struct`);
  return readFields(def.fields, data, { at: 8 }) as T;
}

/** Instruction data: discriminator, then the args in IDL order. */
export function encodeIx(name: string, args: Record<string, unknown>): Uint8Array {
  const ix = IDL.instructions.find((i) => i.name === name);
  if (!ix) throw new Error(`IDL has no instruction ${name}`);
  const out = [...ix.discriminator];
  for (const a of ix.args) {
    if (!(a.name in args)) throw new Error(`${name}: missing arg ${a.name}`);
    write(a.type, args[a.name], out);
  }
  return new Uint8Array(out);
}

/** The IDL name of a custom program error code (Anchor codes start at 6000), if any. */
export function programErrorName(code: number): string | null {
  return IDL.errors?.find((e) => e.code === code)?.name ?? null;
}
