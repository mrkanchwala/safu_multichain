import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  connectSolanaWallet,
  onSolanaWalletRegistered,
  signSolanaMessage,
  signSolanaTransaction,
  solanaWallets,
} from "../../shared/solana-wallet";
import type { SolanaSession } from "../../shared/solana-wallet";
import { WALLET_CHAIN } from "./pool";

// Solana only: any wallet-standard wallet (chains/shared/solana-wallet.ts), on this pool's cluster.
// No WalletConnect.

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

export function ClientProvider({ children }: { children: ReactNode }) {
  const [address, setAddress] = useState<string | null>(null);
  const [walletName, setWalletName] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);
  const solRef = useRef<SolanaSession | null>(null);

  const connect = useCallback(async (walletId?: string) => {
    setConnecting(true);
    try {
      const opt = solanaWallets().find((w) => w.id === walletId) ?? solanaWallets()[0];
      if (!opt) throw new Error("No Solana wallet found. Install Phantom, Solflare or Backpack.");
      const account = await connectSolanaWallet(opt);
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
      connectSolanaWallet(opt, true)
        .then((account) => {
          if (solRef.current) return; // the user connected by hand meanwhile
          solRef.current = { opt, account };
          setAddress(account.address);
          setWalletName(opt.name);
        })
        .catch(() => rememberWallet(null));
    };
    tryReconnect();
    const off = onSolanaWalletRegistered(tryReconnect);
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
    return signSolanaMessage(sol, bytes);
  }, []);

  const solanaSignTransaction = useCallback(async (tx: Uint8Array): Promise<Uint8Array> => {
    const sol = solRef.current;
    if (!sol) throw new Error("Connect a wallet first.");
    return signSolanaTransaction(sol, tx, WALLET_CHAIN);
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
