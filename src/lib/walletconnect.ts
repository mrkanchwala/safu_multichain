// WalletConnect for Ethereum (Sepolia) and Solana (devnet), added 2026-09-25 (founder).
//
// Same pattern the Stellar wallet kit's own WalletConnect module uses (node_modules/@creit.tech/
// stellar-wallets-kit/esm/sdk/modules/wallet-connect.module.js): SignClient owns the session,
// AppKit is only the QR / wallet-list modal (`manualWCControl`). Stellar WalletConnect goes through
// that kit module, not this file.
//
// Loaded with dynamic import() on first use, so its ~2 MB never ships to visitors who don't pick
// WalletConnect. The project id is safustaking.com's (domain-allowlisted, same one /t3 uses; the
// site moved to that domain on 2026-09-25, which is what made this possible without a paid plan).
//
// Signing only: the app never asks the wallet for reads (receipts, balances go through our RPCs),
// which is what WalletConnect wallets reliably support.

import { SignClient } from "@walletconnect/sign-client";
import type { SessionTypes } from "@walletconnect/types";
import { createAppKit } from "@reown/appkit/core";
import { mainnet, sepolia, solana, solanaDevnet } from "@reown/appkit/networks";
import {
  getBase58Decoder,
  getBase58Encoder,
  getBase64Decoder,
  getBase64Encoder,
  getTransactionDecoder,
  getTransactionEncoder,
} from "@solana/kit";
import type { Address, SignatureBytes } from "@solana/kit";
import { EVM_CHAIN_ID } from "./network";
import { IS_MAINNET, POOL } from "./pool";

export const WALLETCONNECT_PROJECT_ID = "3824772bb6c01d55a924dac308a3cb3e";

const EVM_CHAIN = `eip155:${EVM_CHAIN_ID}`;
// Networks follow the pool file (C1, 2026-09-29): Sepolia + Solana devnet on testnet, mainnets on mainnet.
const EVM_NETWORK = IS_MAINNET ? mainnet : sepolia;
const SOL_NETWORK = IS_MAINNET ? solana : solanaDevnet;
const SOL_CHAIN = SOL_NETWORK.caipNetworkId;
// Mainnet ids are ASKED FOR TOO (founder phone test 2026-09-25): a proposal listing only testnets
// is rejected by most phone wallets -- Rainbow "Unsupported chains" (5100), Trust "User rejected"
// (5000) on Solana devnet. Nothing is ever sent on mainnet:
//  * Ethereum still needs Sepolia in the approved session (eth_sendTransaction is chain-bound);
//    without it the user gets a clear "turn on test networks" message.
//  * Solana signatures don't depend on the cluster (the devnet blockhash is inside the signed bytes),
//    so a wallet that only approves mainnet can still sign our devnet transactions.
const EVM_MAINNET = "eip155:1";
const SOL_MAINNETS = ["solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp", "solana:4sGjMW1sUnHzSxGspuhpqLDx6wiyjNtZ"];
const METADATA = {
  name: "SAFU",
  description: "Stake into a SAFU pool and get covered.",
  url: "https://safustaking.com",
  icons: ["https://safustaking.com/favicon.svg"],
};

type Kind = "evm" | "solana";
export type WcSession = { kind: Kind; address: string; topic: string; chainId: string };

let client: Awaited<ReturnType<typeof SignClient.init>> | null = null;
let modal: ReturnType<typeof createAppKit> | null = null;

async function setup() {
  if (!client) {
    // Own storage prefix: the Stellar kit's WalletConnect module runs its own client on this page.
    client = await SignClient.init({ projectId: WALLETCONNECT_PROJECT_ID, metadata: METADATA, customStoragePrefix: "safu-evm-sol" });
  }
  if (!modal) {
    modal = createAppKit({
      projectId: WALLETCONNECT_PROJECT_ID,
      manualWCControl: true,
      networks: [EVM_NETWORK, SOL_NETWORK],
      metadata: METADATA,
      features: { analytics: false, email: false, socials: false, swaps: false, onramp: false },
    });
  }
  return { client, modal };
}

function namespaces(kind: Kind): Record<string, { methods: string[]; chains: string[]; events: string[] }> {
  return kind === "evm"
    ? { eip155: { methods: ["eth_sendTransaction", "personal_sign"], chains: [...new Set([EVM_CHAIN, EVM_MAINNET])], events: ["chainChanged", "accountsChanged"] } }
    : { solana: { methods: ["solana_signTransaction", "solana_signMessage"], chains: [...new Set([SOL_CHAIN, ...SOL_MAINNETS])], events: [] } };
}

/** The account to use and the chain id to send requests on. EVM: the pool's chain only. Solana: pool cluster if
 *  approved, else mainnet (same key, cluster-independent signatures). CAIP-10: `ns:ref:address`. */
function accountFor(session: SessionTypes.Struct, kind: Kind): { address: string; chainId: string } | null {
  const accounts = Object.values(session.namespaces).flatMap((ns) => ns.accounts);
  const order = kind === "evm" ? [EVM_CHAIN] : [...new Set([SOL_CHAIN, ...SOL_MAINNETS])];
  for (const chain of order) {
    const hit = accounts.find((a) => a.startsWith(chain + ":"));
    if (hit) return { address: hit.slice(chain.length + 1), chainId: chain };
  }
  return null;
}

/** Opens the QR modal, waits for the phone wallet to approve. Rejects if the user closes it. */
export async function connect(kind: Kind): Promise<WcSession> {
  const { client: c, modal: m } = await setup();
  const { uri, approval } = await c.connect({ optionalNamespaces: namespaces(kind) });
  if (uri) await m.open({ uri });

  // Closing the modal never rejects `approval()`, so race it against the modal closing.
  let unsubscribe = () => {};
  const closed = new Promise<never>((_, reject) => {
    unsubscribe = m.subscribeState((s) => {
      if (!s.open) reject({ code: -1, message: "The user closed the modal." });
    });
  });
  try {
    const session = await Promise.race([approval(), closed]);
    const hit = accountFor(session, kind);
    if (!hit) {
      void c.disconnect({ topic: session.topic, reason: { code: 6000, message: "Wrong network" } });
      const wallet = session.peer?.metadata?.name || "Your wallet";
      throw new Error(
        kind === "evm"
          ? IS_MAINNET
            ? `Wrong network -- ${wallet} didn't allow ${POOL.evm.label}. Allow it in the wallet and try again.`
            : `Wrong network -- ${wallet} didn't allow ${POOL.evm.short}. Turn on test networks in its settings and try again.`
          : `Wrong network -- ${wallet} didn't share a Solana account.`,
      );
    }
    return { kind, address: hit.address, topic: session.topic, chainId: hit.chainId };
  } finally {
    unsubscribe();
    void m.close();
  }
}

export async function disconnect(s: WcSession): Promise<void> {
  if (!client) return;
  await client.disconnect({ topic: s.topic, reason: { code: 6000, message: "User disconnected" } }).catch(() => {});
}

async function request<T>(s: WcSession, method: string, params: unknown): Promise<T> {
  const { client: c } = await setup();
  return c.request<T>({ topic: s.topic, chainId: s.chainId, request: { method, params } });
}

/** EVM: forwards an EIP-1193 call (only signing methods are ever sent here). */
export function evmRequest(s: WcSession, method: string, params?: unknown[]): Promise<unknown> {
  return request(s, method, params ?? []);
}

/** Solana: sign a wire-format transaction, return the signed wire bytes. Wallets answer either
 *  with the whole signed transaction (base64) or with just our signature (base58, older spec). */
export async function solanaSignTransaction(s: WcSession, tx: Uint8Array): Promise<Uint8Array> {
  const b64 = getBase64Decoder().decode(tx);
  const out = await request<{ transaction?: string; signature?: string }>(s, "solana_signTransaction", {
    transaction: b64,
    pubkey: s.address,
  });
  if (out.transaction) return new Uint8Array(getBase64Encoder().encode(out.transaction));
  if (!out.signature) throw new Error("The wallet returned no signature.");
  const decoded = getTransactionDecoder().decode(tx);
  const signed = {
    ...decoded,
    signatures: { ...decoded.signatures, [s.address as Address]: new Uint8Array(getBase58Encoder().encode(out.signature)) as SignatureBytes },
  };
  return new Uint8Array(getTransactionEncoder().encode(signed));
}

/** Sign raw message bytes. Returns the signature as hex, no 0x (same shape as the extension path). */
export async function signMessage(s: WcSession, bytes: Uint8Array): Promise<string> {
  const hex = Array.from(bytes, (x) => x.toString(16).padStart(2, "0")).join("");
  if (s.kind === "evm") {
    const sig = await request<string>(s, "personal_sign", ["0x" + hex, s.address]);
    return sig.replace(/^0x/, "");
  }
  const out = await request<{ signature: string }>(s, "solana_signMessage", {
    message: getBase58Decoder().decode(bytes),
    pubkey: s.address,
  });
  const sig = getBase58Encoder().encode(out.signature);
  return Array.from(sig, (x) => x.toString(16).padStart(2, "0")).join("");
}
