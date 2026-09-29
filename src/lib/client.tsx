import { createContext, useCallback, useContext, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { StellarWalletsKit, Networks } from "@creit.tech/stellar-wallets-kit";
import { FreighterModule, FREIGHTER_ID } from "@creit.tech/stellar-wallets-kit/modules/freighter";
import { HotWalletModule } from "@creit.tech/stellar-wallets-kit/modules/hotwallet";
import { AlbedoModule } from "@creit.tech/stellar-wallets-kit/modules/albedo";
import { xBullModule } from "@creit.tech/stellar-wallets-kit/modules/xbull";
import { LobstrModule } from "@creit.tech/stellar-wallets-kit/modules/lobstr";
import { RabetModule } from "@creit.tech/stellar-wallets-kit/modules/rabet";
import { HanaModule } from "@creit.tech/stellar-wallets-kit/modules/hana";
import { getWallets } from "@wallet-standard/app";
import type { Wallet, WalletAccount } from "@wallet-standard/base";
import { EVM_CHAIN_ID } from "./network";
import { IS_MAINNET, POOL } from "./pool";

// Stellar wallet network follows the pool file (C1, 2026-09-29).
const STELLAR_WALLET_NETWORK = IS_MAINNET ? Networks.PUBLIC : Networks.TESTNET;
import type { WcSession } from "./walletconnect";

// Wallet scope, 2026-09-25: WalletConnect on all 3 chains plus extension wallets.
// WalletConnect had been kept out because safustaking.com's project id is domain-allowlisted to
// safustaking.com and this app lived elsewhere. It moved to the safustaking.com root on 2026-09-25,
// and the same project id was verified live on /t3 (QR + relay pairing, Ethereum and Stellar).
// Stellar WalletConnect = the kit's own module (loaded on first use, below); Ethereum + Solana
// WalletConnect = lib/walletconnect.ts. Both are dynamic imports: ~2 MB each, only on demand.
// Extensions added: xBull, LOBSTR, Rabet, Hana (Stellar); any wallet-standard Solana wallet.
//
// HOT Wallet added 2026-09-20 (requested: Freighter only supports one active account per
// browser profile, so testing two payout wallets meant re-importing a seed each time).
// CONFIRMED NON-FUNCTIONAL FOR THIS BUILD, same day: HOT Wallet's own Stellar adapter
// (node_modules/@hot-wallet/sdk/src/adapter/stellar.ts) hardcodes getNetwork() to
// "Public Global Stellar Network" with no testnet branch anywhere in the source -- not a default,
// the only value it can ever return. We independently confirmed HOT's own app shows no
// Stellar testnet option at all. Left in the module list anyway (harmless, clearly its own
// product, does not affect Freighter/Albedo) -- pull it before a public submission if an
// unexplained dead option in the connect modal is worse than the polish gain of leaving it.
//
// Albedo added same day as the real fix -- a web-popup signer, no extension install, so testing
// with a different secret key per session needs no profile-juggling. Verified in its own source
// (esm/sdk/modules/albedo.module.js) that it forwards `networkPassphrase` per-transaction to its
// own signing call rather than assuming a fixed network like HOT does, and our own signing call
// (soroban.ts) already always passes NETWORK_PASSPHRASE (testnet) on every request -- confirmed
// end to end, not assumed from the type signature alone.
//
// Real API verified against the published package (unpkg @creit.tech/stellar-wallets-kit@2.6.0,
// esm/sdk/kit.js + esm/sdk/modules/freighter.module.js) 2026-09-17, not written from memory:
// every method on StellarWalletsKit is STATIC (`StellarWalletsKit.init(...)`, not
// `new StellarWalletsKit(...)`), `authModal()` returns `Promise<{ address }>` directly (no
// onWalletSelected callback), and FreighterModule/FREIGHTER_ID live at the
// `/modules/freighter` subpath, not the package root.
const STELLAR_EXTENSIONS = () => [
  new FreighterModule(),
  new xBullModule(),
  new LobstrModule(),
  new RabetModule(),
  new HanaModule(),
  new AlbedoModule(),
  new HotWalletModule(),
];

StellarWalletsKit.init({
  modules: STELLAR_EXTENSIONS(),
  selectedWalletId: FREIGHTER_ID,
  network: STELLAR_WALLET_NETWORK,
});

// Adds the kit's WalletConnect module the first time someone opens the Stellar picker. Re-running
// init() only resets the kit's module list (esm/sdk/kit.js), so this is safe.
let stellarWcReady: Promise<void> | null = null;
function loadStellarWalletConnect(): Promise<void> {
  stellarWcReady ??= (async () => {
    const [{ WalletConnectModule, WalletConnectTargetChain }, { WALLETCONNECT_PROJECT_ID }] = await Promise.all([
      import("@creit.tech/stellar-wallets-kit/modules/wallet-connect"),
      import("./walletconnect"),
    ]);
    const wc = new WalletConnectModule({
      projectId: WALLETCONNECT_PROJECT_ID,
      metadata: {
        name: "SAFU",
        description: "Stake into a SAFU pool and get covered.",
        url: "https://safustaking.com",
        icons: ["https://safustaking.com/favicon.svg"],
      },
      allowedChains: [IS_MAINNET ? WalletConnectTargetChain.PUBLIC : WalletConnectTargetChain.TESTNET],
      // Own storage: lib/walletconnect.ts runs a separate client for Ethereum/Solana.
      signClientOptions: { customStoragePrefix: "safu-stellar" },
    });
    // The module starts its SignClient in the background (constructor, not awaited) and reports
    // "Install" in the picker until that finishes -- wait for it, up to 10 s.
    for (let i = 0; i < 50 && !(await wc.isAvailable()); i++) await new Promise((r) => setTimeout(r, 200));
    StellarWalletsKit.init({
      modules: [
        ...STELLAR_EXTENSIONS(),
        wc,
      ],
      selectedWalletId: FREIGHTER_ID,
      network: STELLAR_WALLET_NETWORK,
    });
  })().catch((e) => {
    stellarWcReady = null; // retry next time; extensions still work
    throw e;
  });
  return stellarWcReady;
}

// --- EVM + Solana, added 2026-09-24 (multichain build) -------------------------------------------------
// EVM: EIP-6963 extension discovery + legacy window.ethereum, the same connection model as
// safustaking.com (website/js/connector-evm.js), plus WalletConnect (lib/walletconnect.ts).
// Solana: any wallet-standard extension that can sign Solana transactions (was limited to
// Phantom/Solflare/Backpack until 2026-09-25), plus WalletConnect (lib/walletconnect.ts).

export type WalletKind = "stellar" | "evm" | "solana";

type Eip1193 = { request: (args: { method: string; params?: unknown[] }) => Promise<unknown> };
export type EvmWalletOption = { id: string; name: string; icon?: string; provider: Eip1193 };
export type SolanaWalletOption = { id: string; name: string; icon?: string; wallet: Wallet };

/** Id used in the picker for the WalletConnect option on Ethereum and Solana. */
export const WALLETCONNECT_ID = "walletconnect";
const EVM_CHAIN_HEX = "0x" + EVM_CHAIN_ID.toString(16);

const evmFound = new Map<string, EvmWalletOption>();
if (typeof window !== "undefined") {
  window.addEventListener("eip6963:announceProvider", ((e: CustomEvent) => {
    const { info, provider } = e.detail as { info: { uuid: string; name: string; icon?: string }; provider: Eip1193 };
    evmFound.set(info.uuid, { id: info.uuid, name: info.name, icon: info.icon, provider });
  }) as EventListener);
  window.dispatchEvent(new Event("eip6963:requestProvider"));
}

export function evmWallets(): EvmWalletOption[] {
  const list = [...evmFound.values()];
  const legacy = (window as unknown as { ethereum?: Eip1193 }).ethereum;
  if (list.length === 0 && legacy) list.push({ id: "injected", name: "Browser wallet", provider: legacy });
  return list;
}

export function solanaWallets(): SolanaWalletOption[] {
  return getWallets()
    .get()
    .filter((w) => "standard:connect" in w.features && "solana:signTransaction" in w.features)
    .filter((w) => w.chains.some((c) => c.startsWith("solana:")))
    .map((w) => ({ id: w.name, name: w.name, icon: w.icon, wallet: w }));
}

async function connectEvm(opt: EvmWalletOption): Promise<string> {
  const accounts = (await opt.provider.request({ method: "eth_requestAccounts" })) as string[];
  if (!accounts?.length) throw new Error("The wallet returned no account.");
  const chain = (await opt.provider.request({ method: "eth_chainId" })) as string;
  if (parseInt(chain, 16) !== EVM_CHAIN_ID) {
    try {
      await opt.provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: EVM_CHAIN_HEX }] });
    } catch {
      throw new Error(`Wrong network. Switch your wallet to ${POOL.evm.label}, then connect again.`);
    }
  }
  return accounts[0];
}

async function connectSolana(opt: SolanaWalletOption): Promise<WalletAccount> {
  const feature = opt.wallet.features["standard:connect"] as {
    connect: () => Promise<{ accounts: readonly WalletAccount[] }>;
  };
  const { accounts } = await feature.connect();
  const acct = accounts.find((a) => a.chains.some((c) => c.startsWith("solana:"))) ?? accounts[0];
  if (!acct) throw new Error("The wallet returned no account.");
  return acct;
}

export type AppClient = {
  /** Which chain family the connected wallet is on; null when nothing is connected. */
  kind: WalletKind | null;
  /** The wallet's own address on its chain (G..., 0x..., or a base58 Solana key). */
  address: string | null;
  walletName: string | null;
  connecting: boolean;
  connect: (kind: WalletKind, walletId?: string) => Promise<void>;
  disconnect: () => void;
  /** Sign a UTF-8 message (EVM personal_sign / Solana signMessage). Returns hex, no 0x. */
  signMessage: (message: string | Uint8Array) => Promise<string>;
  /** EVM only: a raw EIP-1193 request through the connected wallet. */
  evmRequest: (method: string, params?: unknown[]) => Promise<unknown>;
  /** Solana only: the wallet signs a serialized (wire-format) transaction; returns the signed bytes. */
  solanaSignTransaction: (tx: Uint8Array) => Promise<Uint8Array>;
};

const ClientContext = createContext<AppClient | null>(null);

const toHex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

export function ClientProvider({ children }: { children: ReactNode }) {
  const [kind, setKind] = useState<WalletKind | null>(null);
  const [address, setAddress] = useState<string | null>(null);
  const [walletName, setWalletName] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);
  const evmRef = useRef<EvmWalletOption | null>(null);
  const solRef = useRef<{ opt: SolanaWalletOption; account: WalletAccount } | null>(null);
  const wcRef = useRef<WcSession | null>(null);

  const connect = useCallback(async (k: WalletKind, walletId?: string) => {
    setConnecting(true);
    try {
      if (k !== "stellar" && walletId === WALLETCONNECT_ID) {
        const wc = await import("./walletconnect");
        const session = await wc.connect(k);
        wcRef.current = session;
        setAddress(session.address);
        setWalletName("WalletConnect");
      } else if (k === "stellar") {
        // WalletConnect joins the list on first open; if it fails to load, extensions still work.
        await loadStellarWalletConnect().catch(() => {});
        const { address: addr } = await StellarWalletsKit.authModal();
        setAddress(addr);
        setWalletName("Stellar wallet");
      } else if (k === "evm") {
        const opt = evmWallets().find((w) => w.id === walletId) ?? evmWallets()[0];
        if (!opt) throw new Error("No EVM wallet found. Install MetaMask, Rabby or another browser wallet.");
        const addr = await connectEvm(opt);
        evmRef.current = opt;
        setAddress(addr);
        setWalletName(opt.name);
      } else {
        const opt = solanaWallets().find((w) => w.id === walletId) ?? solanaWallets()[0];
        if (!opt) throw new Error("No Solana wallet found. Install Phantom, Solflare or Backpack.");
        const account = await connectSolana(opt);
        solRef.current = { opt, account };
        setAddress(account.address);
        setWalletName(opt.name);
      }
      setKind(k);
    } finally {
      setConnecting(false);
    }
  }, []);

  const disconnect = useCallback(() => {
    if (kind === "stellar") StellarWalletsKit.disconnect();
    const sol = solRef.current;
    const dis = sol?.opt.wallet.features["standard:disconnect"] as { disconnect?: () => Promise<void> } | undefined;
    if (dis?.disconnect) void dis.disconnect();
    const wc = wcRef.current;
    if (wc) void import("./walletconnect").then((m) => m.disconnect(wc));
    wcRef.current = null;
    evmRef.current = null;
    solRef.current = null;
    setKind(null);
    setAddress(null);
    setWalletName(null);
  }, [kind]);

  const signMessage = useCallback(
    async (message: string | Uint8Array): Promise<string> => {
      const bytes = typeof message === "string" ? new TextEncoder().encode(message) : message;
      if (wcRef.current) return (await import("./walletconnect")).signMessage(wcRef.current, bytes);
      if (kind === "evm" && evmRef.current && address) {
        const sig = (await evmRef.current.provider.request({
          method: "personal_sign",
          params: ["0x" + toHex(bytes), address],
        })) as string;
        return sig.replace(/^0x/, "");
      }
      if (kind === "solana" && solRef.current) {
        const feature = solRef.current.opt.wallet.features["solana:signMessage"] as
          | { signMessage: (...i: { account: WalletAccount; message: Uint8Array }[]) => Promise<{ signature: Uint8Array }[]> }
          | undefined;
        if (!feature) throw new Error("This wallet can't sign SAFU requests. For Solana, use Phantom or Solflare.");
        const [out] = await feature.signMessage({ account: solRef.current.account, message: bytes });
        return toHex(out.signature);
      }
      throw new Error("Message signing is only used for EVM and Solana wallets here.");
    },
    [kind, address],
  );

  const evmRequest = useCallback(async (method: string, params?: unknown[]) => {
    if (wcRef.current?.kind === "evm") return (await import("./walletconnect")).evmRequest(wcRef.current, method, params);
    if (!evmRef.current) throw new Error("Connect an Ethereum wallet first.");
    return evmRef.current.provider.request({ method, params });
  }, []);

  const solanaSignTransaction = useCallback(async (tx: Uint8Array): Promise<Uint8Array> => {
    if (wcRef.current?.kind === "solana") return (await import("./walletconnect")).solanaSignTransaction(wcRef.current, tx);
    const sol = solRef.current;
    if (!sol) throw new Error("Connect a Solana wallet first.");
    const feature = sol.opt.wallet.features["solana:signTransaction"] as
      | { signTransaction: (...i: { account: WalletAccount; transaction: Uint8Array; chain?: string }[]) => Promise<{ signedTransaction: Uint8Array }[]> }
      | undefined;
    if (!feature) throw new Error("This wallet can't sign Solana transactions. Use Phantom or Solflare.");
    const [out] = await feature.signTransaction({ account: sol.account, transaction: tx, chain: POOL.solana.wallet_chain });
    return out.signedTransaction;
  }, []);

  const value = useMemo(
    () => ({ kind, address, walletName, connecting, connect, disconnect, signMessage, evmRequest, solanaSignTransaction }),
    [kind, address, walletName, connecting, connect, disconnect, signMessage, evmRequest, solanaSignTransaction],
  );

  return <ClientContext.Provider value={value}>{children}</ClientContext.Provider>;
}

export function useClient<T = AppClient>(): T {
  const ctx = useContext(ClientContext);
  if (!ctx) throw new Error("useClient must be used inside ClientProvider");
  return ctx as T;
}

/** The connected STELLAR wallet as `{ address }`, or null. The Stellar-direct panels sign with the
 *  wallet kit, so an EVM/Solana connection is deliberately not a "payer" here -- those users act
 *  through their safu-account and the relayer (Stage 2 Steps 3-6). */
export function usePayer(client: AppClient): { address: string } | null {
  return client.kind === "stellar" && client.address ? { address: client.address } : null;
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

/** Wraps any async function in running/error/success state. Stage 1 passes stubs
 *  (lib/actions.ts); Stage 6 swaps them for real Soroban calls without this hook or any component
 *  needing to change. */
export function useAction<T = string>(fn: (signal?: AbortSignal) => Promise<T>): ActionState<T> {
  const [isRunning, setIsRunning] = useState(false);
  const [isError, setIsError] = useState(false);
  const [isSuccess, setIsSuccess] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const [data, setData] = useState<T | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const reset = useCallback(() => {
    setIsError(false);
    setIsSuccess(false);
    setError(null);
    setData(null);
  }, []);

  const dispatch = useCallback(() => {
    const controller = new AbortController();
    abortRef.current = controller;
    setIsRunning(true);
    setIsError(false);
    setIsSuccess(false);
    fn(controller.signal)
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
