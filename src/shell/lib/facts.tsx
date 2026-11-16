import { createContext, useContext } from "react";
import type { ReactNode } from "react";

/** Facts about the active pool, read from its contract by the chain adapter (B42 review A2).
 *  Copy names a fact; it never types the value. Unknown until the read returns. */
export type PoolFacts = { yieldVaultUrl?: string };

export const PoolFactsContext = createContext<PoolFacts>({});

/** A link whose target is a pool fact: plain text until the fact is known. */
export function FactLink({ fact, children }: { fact: keyof PoolFacts; children: ReactNode }) {
  const href = useContext(PoolFactsContext)[fact];
  if (!href) return <>{children}</>;
  return (
    <a href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  );
}
