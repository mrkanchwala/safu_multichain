import { useEffect, useState } from "react";
import type { AppClient } from "./client";
import { safuAccountFor } from "./crosschain";
import type { Mode } from "./crosschain";

/** The address the pool knows this user by, for staking or for backing.
 *  Stellar wallet: the wallet itself. EVM/Solana wallet: its safu-account from that mode's adapter
 *  (deterministic, so it's known before the first deposit creates it). */
export function useStaker(client: AppClient, mode: Mode): { address: string | null; crossChain: boolean } {
  const [account, setAccount] = useState<string | null>(null);
  const crossChain = client.kind === "evm" || client.kind === "solana";

  useEffect(() => {
    let cancelled = false;
    setAccount(null);
    if ((client.kind === "evm" || client.kind === "solana") && client.address) {
      safuAccountFor(client.kind, client.address, mode)
        .then((a) => !cancelled && setAccount(a))
        .catch(() => !cancelled && setAccount(null));
    }
    return () => {
      cancelled = true;
    };
  }, [client.kind, client.address, mode]);

  if (client.kind === "stellar") return { address: client.address, crossChain: false };
  return { address: crossChain ? account : null, crossChain };
}
