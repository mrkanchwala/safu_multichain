import { SITE } from "../shell/lib/site";
import type { ChainAdapter } from "./types";

// Build constant from vite.config.ts: true only when hq/site.toml enables Solana.
declare const __CHAIN_SOLANA__: boolean;

// One loader per pool, so each chain's wallet and contract code is its own chunk, fetched only for
// the chain being shown. Solana's entry exists only when enabled, so a disabled chain's code never
// reaches the public bundle (the import below is dead code then).
const LOADERS: Record<string, () => Promise<ChainAdapter>> = {
  stellar: () => import("./stellar/adapter").then((m) => m.stellar),
  ...(__CHAIN_SOLANA__ ? { solana: () => import("./solana/adapter").then((m) => m.solana) } : {}),
};

export const ADAPTER_IDS: readonly string[] = Object.keys(LOADERS);

/** The chain a URL asks for (`?chain=`) when it is enabled; otherwise the site's default. */
export function chainFromSearch(
  search: string,
  enabled: readonly string[] = SITE.chains.enabled,
  fallback: string = SITE.chains.default,
): string {
  const asked = new URLSearchParams(search).get("chain");
  return asked && enabled.includes(asked) ? asked : fallback;
}

export function loadAdapter(search: string): Promise<ChainAdapter> {
  const id = chainFromSearch(search);
  const load = LOADERS[id];
  if (!load) throw new Error(`hq/site.toml enables "${id}" but no adapter exists for it`);
  return load();
}
