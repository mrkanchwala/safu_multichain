import { useEffect, useState } from "react";
import { readStakeBounds } from "./reads";
import type { StakeBounds } from "./network";

/** The pool's live stake limits; null while loading or if the read failed (nothing then claims a number). */
export function useStakeBounds(): StakeBounds | null {
  const [bounds, setBounds] = useState<StakeBounds | null>(null);
  useEffect(() => {
    let cancelled = false;
    readStakeBounds().then((b) => !cancelled && setBounds(b), () => {});
    return () => {
      cancelled = true;
    };
  }, []);
  return bounds;
}
