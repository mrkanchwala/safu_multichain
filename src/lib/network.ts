// Network settings. Renamed from market.ts 2026-09-18:
// there is no lending market in this build. The pool is a fork of SAFU's live
// protection-pool, USDC-denominated, covering wallet drains and wrongful
// liquidations across three chains. Treasury/USTRY/RWA collateral belonged to the dropped money market
// and is dead; do not reintroduce it here or anywhere else in this build.
// EVERY network value below comes from the pool's network file (C1, 2026-09-29): one switch,
// SAFU_POOL_NETWORK, picks config/pool.<network>.json at build time (pool.ts). Do not hard-code
// an address, URL, chain id or label here or anywhere downstream.
import { Asset } from "@stellar/stellar-sdk";
import { POOL } from "./pool";

export const CLUSTER = POOL.network;
export const HORIZON_URL = POOL.stellar.horizon;
export const SOROBAN_RPC_URL = POOL.stellar.soroban_rpc;
export const NETWORK_PASSPHRASE = POOL.stellar.passphrase;

// Single contract: the protection pool. Ids are written into the pool file by deploy_v1.sh.
// See soroban.ts's isLive().
export const POOL_CONTRACT_ID: string = POOL.contracts.pool;
// CCTP adapters: an EVM/Solana wallet's USDC lands here and becomes a stake / a backing.
export const STAKE_ADAPTER_ID = POOL.contracts.stake_adapter;
export const BACK_ADAPTER_ID = POOL.contracts.back_adapter;
export const USDC_ASSET_CODE = "USDC";

// Classic Stellar assets are always 7 decimals (the protocol's stroop scale),
// not configurable per-asset. USDC on Stellar is a classic asset, not a
// 6-decimal token -- a 6 here silently renders every figure 10x too large
// (the same bug class the USTRY/8-decimals mistake was on 2026-09-17). Do not
// "correct" this back.
export const USDC_DECIMALS = 7;

// Stake bounds -- REAL contract constants, not chosen for the frontend.
// `stake.rs`: min_stake/max_stake = pool_cap * BPS / 10_000, recomputed live on
// every call so a pool-cap change takes effect with no re-anchoring step.
// SET 2026-09-19 : $10 min / $100 max at the $100,000
// deploy target -- no longer V8's ratio (types.rs comment, verbatim).
export const MIN_STAKE_BPS = 1; // 0.01% of pool cap -- $10 at a $100,000 pool cap
export const MAX_STAKE_BPS = 10; // 0.1% of pool cap -- $100 at a $100,000 pool cap
export const STAKE_BPS_DENOMINATOR = 10_000;

// Pool cap for THIS deploy -- set at initialize time (admin.rs's set_pool_cap), not a protocol
// constant. From the pool file (testnet fast-clock: $10,000 -> $1 min / $10 max stake).
export const POOL_CAP_USDC = POOL.pool.cap_usdc;
export const MIN_STAKE_USDC = (POOL_CAP_USDC * MIN_STAKE_BPS) / STAKE_BPS_DENOMINATOR;
export const MAX_STAKE_USDC = (POOL_CAP_USDC * MAX_STAKE_BPS) / STAKE_BPS_DENOMINATOR;

// Mirrors types.rs's YIELD_INDEX_PRECISION exactly (A7, 2026-09-18) -- the
// 1.0x starting value for the yield-index ratio, needed here to render
// "how much has this grown" rather than a raw contract-scale integer.
export const YIELD_INDEX_PRECISION = 1_000_000_000_000;

// The pool's asset is Circle's USDC (what CCTP mints). Its Stellar Asset Contract id is derived from
// issuer + passphrase; backend/tests/test_pool_net.py re-derives it for both networks.
export const usdcContractId = (): string => POOL.stellar.usdc_sac;
export const CIRCLE_USDC = new Asset("USDC", POOL.stellar.usdc_issuer);

// Demo claim API (backend/app.py, 2026-09-19) -- the first live HTTP surface
// in this repo. Dev server proxies /claim to this port (vite.config.ts), so
// this only matters for a non-proxied deployment.
export const CLAIM_API_URL = "http://localhost:8000";

// Shared between StakePanel (covered-wallet registration) and the claim
// filing form -- both MUST use the same chain id strings, since the backend
// registry's reverse lookup matches on the exact string stored at
// registration time. Ethereum's id is "sepolia" on testnet, "eth" on mainnet (pool file).
export type ChainId = "sepolia" | "eth" | "solana" | "stellar";
export const EVM_CHAIN_KEY: ChainId = POOL.evm.chain_key;
export const CHAINS: readonly { id: ChainId; label: string }[] = [
  { id: POOL.evm.chain_key, label: POOL.evm.label },
  { id: "solana", label: POOL.solana.label },
  { id: "stellar", label: POOL.stellar.label },
];

// --- CCTP (Circle) ---------------------------------------------------------------------------
export const CCTP_DOMAIN = POOL.cctp.domains;
export const EVM_CHAIN_ID = POOL.evm.chain_id;
export const EVM_RPC_URL = POOL.evm.public_rpc;
export const EVM_USDC = POOL.evm.usdc;
export const EVM_TOKEN_MESSENGER_V2 = POOL.evm.token_messenger_v2;
export const EVM_MESSAGE_TRANSMITTER_V2 = POOL.evm.message_transmitter_v2;
export const SOLANA_RPC_URL = POOL.solana.public_rpc;
export const SOLANA_USDC_MINT = POOL.solana.usdc_mint;
export const SOLANA_MESSAGE_TRANSMITTER = POOL.solana.message_transmitter_v2;
export const SOLANA_TOKEN_MESSENGER_MINTER = POOL.solana.token_messenger_minter_v2;
export const CCTP_FINALITY_STANDARD = 2000;
// Deposits use Fast Transfer (2026-09-24): seconds instead of ~13-19 min of Ethereum finality.
// Circle charges 1 bp on these routes; every burn allows 10 bp (amount / 1000).
export const CCTP_FINALITY_FAST = 1000;
