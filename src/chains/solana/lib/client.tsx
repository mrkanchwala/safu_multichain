import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { getWallets } from "@wallet-standard/app";
import type { Wallet, WalletAccount } from "@wallet-standard/base";
import { WALLET_CHAIN } from "./pool";

// Solana only: any wallet-standard extension that can sign Solana transactions (browser extensions,
// and the in-app browsers of phone wallets such as Phantom and Solflare). No WalletConnect. From the multichain site's
// lib/client.tsx with the Stellar and Ethereum halves removed.

export type SolanaWalletOption = { id: string; name: string; icon?: string; wallet: Wallet };

export function solanaWallets(): SolanaWalletOption[] {
  return getWallets()
    .get()
    .filter((w) => "standard:connect" in w.features && "solana:signTransaction" in w.features)
    .filter((w) => w.chains.some((c) => c.startsWith("solana:")))
    .map((w) => ({ id: w.name, name: w.name, icon: w.icon, wallet: w }));
}

// The last wallet connected here, so a reload reconnects to it without a popup (silent connect).
const LAST_WALLET_KEY = "safu-solana:last-wallet";

function rememberWallet(id: string | null) {
  try {
    if (id) localStorage.setItem(LAST_WALLET_KEY, id);
    else localStorage.removeItem(LAST_WALLET_KEY);
  } catch {
    // storage blocked (private window): the user just reconnects by hand
  }
}

function lastWallet(): string | null {
  try {
    return localStorage.getItem(LAST_WALLET_KEY);
  } catch {
    return null;
  }
}

async function connectExtension(opt: SolanaWalletOption, silent = false): Promise<WalletAccount> {
  const feature = opt.wallet.features["standard:connect"] as {
    connect: (input?: { silent?: boolean }) => Promise<{ accounts: readonly WalletAccount[] }>;
  };
  const { accounts } = await feature.connect(silent ? { silent: true } : undefined);
  const acct = accounts.find((a) => a.chains.some((c) => c.startsWith("solana:"))) ?? accounts[0];
  if (!acct) throw new Error("The wallet returned no account.");
  return acct;
}

export type AppClient = {
  /** The connected wallet's address (base58); null when nothing is connected. */
  address: string | null;
  walletName: string | null;
  connecting: boolean;
  connect: (walletId?: string) => Promise<void>;
  disconnect: () => void;
  /** Sign a UTF-8 message (Solana signMessage). Returns hex, no 0x. */
  signMessage: (message: string | Uint8Array) => Promise<string>;
  /** The wallet signs a serialized (wire-format) transaction; returns the signed bytes. */
  solanaSignTransaction: (tx: Uint8Array) => Promise<Uint8Array>;
};

const ClientContext = createContext<AppClient | null>(null);

const toHex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

export function ClientProvider({ children }: { children: ReactNode }) {
  const [address, setAddress] = useState<string | null>(null);
  const [walletName, setWalletName] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);
  const solRef = useRef<{ opt: SolanaWalletOption; account: WalletAccount } | null>(null);

  const connect = useCallback(async (walletId?: string) => {
    setConnecting(true);
    try {
      const opt = solanaWallets().find((w) => w.id === walletId) ?? solanaWallets()[0];
      if (!opt) throw new Error("No Solana wallet found. Install Phantom, Solflare or Backpack.");
      const account = await connectExtension(opt);
      solRef.current = { opt, account };
      setAddress(account.address);
      setWalletName(opt.name);
      rememberWallet(opt.id);
    } finally {
      setConnecting(false);
    }
  }, []);

  // After a reload: reconnect to the remembered wallet silently once its extension registers.
  // A wallet that no longer trusts this site returns no account; then the user connects by hand.
  useEffect(() => {
    const id = lastWallet();
    if (!id) return;
    let done = false;
    const tryReconnect = () => {
      const opt = solanaWallets().find((w) => w.id === id);
      if (!opt || done) return;
      done = true;
      connectExtension(opt, true)
        .then((account) => {
          if (solRef.current) return; // the user connected by hand meanwhile
          solRef.current = { opt, account };
          setAddress(account.address);
          setWalletName(opt.name);
        })
        .catch(() => rememberWallet(null));
    };
    tryReconnect();
    const off = getWallets().on("register", tryReconnect);
    return () => {
      done = true;
      off();
    };
  }, []);

  const disconnect = useCallback(() => {
    const dis = solRef.current?.opt.wallet.features["standard:disconnect"] as { disconnect?: () => Promise<void> } | undefined;
    if (dis?.disconnect) void dis.disconnect();
    rememberWallet(null);
    solRef.current = null;
    setAddress(null);
    setWalletName(null);
  }, []);

  const signMessage = useCallback(async (message: string | Uint8Array): Promise<string> => {
    const bytes = typeof message === "string" ? new TextEncoder().encode(message) : message;
    const sol = solRef.current;
    if (!sol) throw new Error("Connect a wallet first.");
    const feature = sol.opt.wallet.features["solana:signMessage"] as
      | { signMessage: (...i: { account: WalletAccount; message: Uint8Array }[]) => Promise<{ signature: Uint8Array }[]> }
      | undefined;
    if (!feature) throw new Error("This wallet can't sign SAFU requests. Use Phantom or Solflare.");
    const [out] = await feature.signMessage({ account: sol.account, message: bytes });
    return toHex(out.signature);
  }, []);

  const solanaSignTransaction = useCallback(async (tx: Uint8Array): Promise<Uint8Array> => {
    const sol = solRef.current;
    if (!sol) throw new Error("Connect a wallet first.");
    const feature = sol.opt.wallet.features["solana:signTransaction"] as
      | { signTransaction: (...i: { account: WalletAccount; transaction: Uint8Array; chain?: string }[]) => Promise<{ signedTransaction: Uint8Array }[]> }
      | undefined;
    if (!feature) throw new Error("This wallet can't sign Solana transactions. Use Phantom or Solflare.");
    const [out] = await feature.signTransaction({ account: sol.account, transaction: tx, chain: WALLET_CHAIN });
    return out.signedTransaction;
  }, []);

  const value = useMemo(
    () => ({ address, walletName, connecting, connect, disconnect, signMessage, solanaSignTransaction }),
    [address, walletName, connecting, connect, disconnect, signMessage, solanaSignTransaction],
  );

  return <ClientContext.Provider value={value}>{children}</ClientContext.Provider>;
}

export function useClient(): AppClient {
  const ctx = useContext(ClientContext);
  if (!ctx) throw new Error("useClient must be used inside ClientProvider");
  return ctx;
}

type ActionState<T> = {
  isRunning: boolean;
  isError: boolean;
  isSuccess: boolean;
  error: unknown;
  data: T | null;
  dispatch: () => void;
  reset: () => void;
};

/** Wraps any async function in running/error/success state. */
export function useAction<T = string>(fn: () => Promise<T>): ActionState<T> {
  const [isRunning, setIsRunning] = useState(false);
  const [isError, setIsError] = useState(false);
  const [isSuccess, setIsSuccess] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [data, setData] = useState<T | null>(null);

  const reset = useCallback(() => {
    setIsError(false);
    setIsSuccess(false);
    setError(null);
    setData(null);
  }, []);

  const dispatch = useCallback(() => {
    setIsRunning(true);
    setIsError(false);
    setIsSuccess(false);
    fn()
      .then((result) => {
        setData(result);
        setIsSuccess(true);
      })
      .catch((err) => {
        setError(err);
        setIsError(true);
      })
      .finally(() => setIsRunning(false));
  }, [fn]);

  return { isRunning, isError, isSuccess, error, data, dispatch, reset };
}
