// The pool's network file, baked in at build time from config/pool.<SAFU_POOL_NETWORK>.json
// (vite.config.ts). Read values from here or from network.ts, never hard-code a network value.
declare const __POOL__: PoolConfig;

export interface PoolConfig {
  network: "testnet" | "mainnet";
  stellar: { label: string; short: string; passphrase: string; soroban_rpc: string; horizon: string;
    usdc_issuer: string; usdc_sac: string; explorer_tx: string };
  evm: { chain_key: "sepolia" | "eth"; label: string; short: string; chain_id: number; public_rpc: string;
    usdc: `0x${string}`; token_messenger_v2: `0x${string}`; message_transmitter_v2: `0x${string}`; explorer_tx: string };
  solana: { label: string; short: string; cluster: "devnet" | "mainnet-beta"; wallet_chain: `solana:${string}`;
    public_rpc: string; usdc_mint: string; message_transmitter_v2: string; token_messenger_minter_v2: string;
    explorer_tx: string };
  cctp: { iris_url: string; domains: { evm: number; solana: number; stellar: number } };
  pool: { cap_usdc: number };
  contracts: { pool: string; registry: string; stake_adapter: string; back_adapter: string; deploy_record: string };
}

export const POOL: PoolConfig = __POOL__;
export const IS_MAINNET = POOL.network === "mainnet";
export const explorerTx = (template: string, tx: string): string => template.replace("{tx}", tx);
