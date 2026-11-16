import { getWallets } from "@wallet-standard/app";
import type { Wallet, WalletAccount } from "@wallet-standard/base";

// Solana wallet-standard, ONE copy for every pool that talks to a Solana wallet (CIE H3, B42): the Stellar
// pool for CCTP stakes, the Solana pool for everything. Any extension (or phone wallet's in-app browser)
// that can connect and sign Solana transactions shows up. Which cluster a transaction is for is the
// caller's: it passes its own wallet chain id.

export type SolanaWalletOption = { id: string; name: string; icon?: string; wallet: Wallet };
/** A connected wallet: the option picked and the account it returned. */
export type SolanaSession = { opt: SolanaWalletOption; account: WalletAccount };

export const toHex = (b: Uint8Array): string => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

export function solanaWallets(): SolanaWalletOption[] {
  return getWallets()
    .get()
    .filter((w) => "standard:connect" in w.features && "solana:signTransaction" in w.features)
    .filter((w) => w.chains.some((c) => c.startsWith("solana:")))
    .map((w) => ({ id: w.name, name: w.name, icon: w.icon, wallet: w }));
}

/** Calls `fn` whenever a wallet extension registers after page load (a silent reconnect waits for it). */
export function onSolanaWalletRegistered(fn: () => void): () => void {
  return getWallets().on("register", fn);
}

/** `silent`: reconnect a wallet the user already approved, without a popup. */
export async function connectSolanaWallet(opt: SolanaWalletOption, silent = false): Promise<WalletAccount> {
  const feature = opt.wallet.features["standard:connect"] as {
    connect: (input?: { silent?: boolean }) => Promise<{ accounts: readonly WalletAccount[] }>;
  };
  const { accounts } = await feature.connect(silent ? { silent: true } : undefined);
  const acct = accounts.find((a) => a.chains.some((c) => c.startsWith("solana:"))) ?? accounts[0];
  if (!acct) throw new Error("The wallet returned no account.");
  return acct;
}

/** Signs a message; returns the signature as hex, no 0x. */
export async function signSolanaMessage(session: SolanaSession, message: Uint8Array): Promise<string> {
  const feature = session.opt.wallet.features["solana:signMessage"] as
    | { signMessage: (...i: { account: WalletAccount; message: Uint8Array }[]) => Promise<{ signature: Uint8Array }[]> }
    | undefined;
  if (!feature) throw new Error("This wallet can't sign SAFU requests. Use Phantom or Solflare.");
  const [out] = await feature.signMessage({ account: session.account, message });
  return toHex(out.signature);
}

/** The wallet signs a serialized (wire-format) transaction for `chain` (e.g. "solana:devnet"). */
export async function signSolanaTransaction(session: SolanaSession, tx: Uint8Array, chain: string): Promise<Uint8Array> {
  const feature = session.opt.wallet.features["solana:signTransaction"] as
    | { signTransaction: (...i: { account: WalletAccount; transaction: Uint8Array; chain?: string }[]) => Promise<{ signedTransaction: Uint8Array }[]> }
    | undefined;
  if (!feature) throw new Error("This wallet can't sign Solana transactions. Use Phantom or Solflare.");
  const [out] = await feature.signTransaction({ account: session.account, transaction: tx, chain });
  return out.signedTransaction;
}
