import { SITE } from "../shell/lib/site";
import type { ChainAdapter } from "./types";

// One loader per pool, so each chain's wallet and contract code is its own chunk, fetched only for
// the chain being shown. B42 Step 2 adds Solana here, gated on hq/site.toml so a disabled chain's
// code never reaches the public bundle.
const LOADERS: Record<string, () => Promise<ChainAdapter>> = {
  stellar: () => import("./stellar/adapter").then((m) => m.stellar),
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
