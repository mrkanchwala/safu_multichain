import { useEffect, useState } from "react";
import { chainNow } from "./program";
import { readPool } from "./reads";
import type { PoolRecord } from "./reads";
import { useTick } from "../../../shell/lib/useTick";

/** The pool record and the chain's clock, re-read on every tick and whenever `refreshKey` changes.
 *  `now` is the chain's time: use it for every "ready at" comparison. */
export function usePool(refreshKey = 0): { pool: PoolRecord | null; error: boolean; now: number } {
  const [pool, setPool] = useState<PoolRecord | null>(null);
  const [error, setError] = useState(false);
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const tick = useTick(5_000);
  useEffect(() => {
    let cancelled = false;
    Promise.all([readPool(), chainNow()])
      .then(([p, t]) => {
        if (cancelled) return;
        setPool(p);
        setError(!p);
        setNow(t);
      })
      .catch(() => !cancelled && setError(true));
    return () => {
      cancelled = true;
    };
  }, [refreshKey, tick]);
  return { pool, error, now };
}
