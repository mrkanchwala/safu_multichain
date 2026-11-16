import { isAddress } from "@solana/kit";
import { StrKey } from "@stellar/stellar-sdk";
import { CHAINS, EVM_CHAIN_KEY, type ChainId } from "./network";

/** Which chain this address is shaped for, or null. */
export function addressChain(wallet: string): ChainId | null {
  const w = wallet.trim();
  if (/^0x[0-9a-fA-F]{40}$/.test(w)) return EVM_CHAIN_KEY;
  if (StrKey.isValidEd25519PublicKey(w) || StrKey.isValidContract(w) || StrKey.isValidMed25519PublicKey(w)) return "stellar";
  if (isAddress(w)) return "solana";
  return null;
}

const label = (id: ChainId) => CHAINS.find((c) => c.id === id)?.label ?? id;

/**
 * Null if `wallet` fits the chain picked, else a message saying which chain it fits.
 * A Solana address added with Ethereum still selected used to come back as
 * "the network isn't responding" (live user, 29 Oct).
 */
export function walletChainMismatch(chain: ChainId, wallet: string): string | null {
  const found = addressChain(wallet);
  const family = chain === "eth" || chain === "sepolia" ? EVM_CHAIN_KEY : chain;
  if (found === family) return null;
  if (found === null) return `That isn't a valid ${label(chain)} address.`;
  return `That's a ${label(found)} address, so pick ${label(found)} above.`;
}
