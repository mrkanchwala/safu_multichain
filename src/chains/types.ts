import type { ComponentType, ReactNode } from "react";
import type { PoolFacts } from "../shell/lib/facts";
import type { FaqOverrides } from "../shell/components/Faq";

export type PoolTab = "stake" | "back" | "file" | "collect";

/** Everything the shell needs from one pool's chain code (B42). The shell owns layout and copy that
 *  is the same for every pool; the adapter owns wallets, contract calls and pool-specific parts. */
export interface ChainAdapter {
  id: string;
  /** Wallet + RPC client context for this pool's components. */
  Provider: ComponentType<{ children: ReactNode }>;
  /** The header's wallet button. */
  Wallet: ComponentType;
  /** A notice above the header (network not live, pool unreachable), or nothing. */
  Badge: ComponentType;
  /** The pool sentence under the shared headline. */
  Hero: ComponentType;
  /** One component per dapp tab. */
  panels: Record<PoolTab, ComponentType>;
  /** The "The pool" numbers section. */
  Numbers: ComponentType;
  /** Facts the copy links to or quotes, read from the pool's contract. */
  useFacts: () => PoolFacts;
  /** FAQ answers where this pool's rules differ from the default answers (null hides a question). */
  faq?: FaqOverrides;
  /** A line above the whitepaper and protocols pages when they describe a different pool, or nothing. */
  ExplainerNote?: ComponentType;
}
