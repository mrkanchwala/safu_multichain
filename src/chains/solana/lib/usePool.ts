import { useEffect, useSyncExternalStore } from "react";
import { chainNow } from "./program";
import { readPool } from "./reads";
import type { PoolRecord } from "./reads";

// ONE poller for the Solana pool (CIE P-1, B42). Every component that shows pool data (badge, numbers, the open
// panel, each live FAQ duration) reads the same snapshot, so a visitor costs one pool read + one clock read per
// tick, not one per component. The poll runs only while something on the page is reading it.

type Snapshot = { pool: PoolRecord | null; error: boolean; now: number };

const POLL_MS = 5_000;
let snapshot: Snapshot = { pool: null, error: false, now: Math.floor(Date.now() / 1000) };
const readers = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | null = null;
let reading = false;
let again = false;

function read(): void {
  if (reading) {
    again = true; // a refresh asked for mid-read reads once more after it, so it sees the new state
    return;
  }
  reading = true;
  Promise.all([readPool(), chainNow()])
    .then(([pool, now]) => {
      snapshot = { pool, error: !pool, now };
    })
    .catch(() => {
      snapshot = { ...snapshot, error: true };
    })
    .finally(() => {
      reading = false;
      readers.forEach((notify) => notify());
      if (again) {
        again = false;
        read();
      }
    });
}

function subscribe(notify: () => void): () => void {
  readers.add(notify);
  if (!timer) {
    read();
    timer = setInterval(read, POLL_MS);
  }
  return () => {
    readers.delete(notify);
    if (readers.size === 0 && timer) {
      clearInterval(timer);
      timer = null;
    }
  };
}

/** The pool record and the chain's clock, shared by every reader. A new `refreshKey` (after a transaction)
 *  re-reads at once. `now` is the chain's time: use it for every "ready at" comparison. */
export function usePool(refreshKey = 0): Snapshot {
  const snap = useSyncExternalStore(subscribe, () => snapshot);
  useEffect(() => {
    if (refreshKey) read();
  }, [refreshKey]);
  return snap;
}
