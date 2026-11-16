import { useEffect, useState } from "react";
import { readStakeBoundsShared } from "./reads";
import type { StakeBounds } from "./network";

/** Wait before re-reading after a failed read (every RPC unreachable). */
const RETRY_MS = 10_000;

/** The pool's live stake limits; null while loading or while the read keeps failing (nothing then claims a
 *  number, and staking stays off). A failed read retries, so one bad moment doesn't leave the tab blank. */
export function useStakeBounds(): StakeBounds | null {
  const [bounds, setBounds] = useState<StakeBounds | null>(null);
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = () =>
      readStakeBoundsShared().then(
        (b) => !cancelled && setBounds(b),
        () => {
          if (!cancelled) timer = setTimeout(load, RETRY_MS);
        },
      );
    load();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, []);
  return bounds;
}
