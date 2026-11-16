// The site's ONE JSON POST to its own APIs (CIE P-3, B42), shared by every pool: each call has a timeout,
// so a backend that hangs ends in a message the page can show instead of a button that spins forever.
// Timeouts sit a little above nginx's read timeout for the route (deploy/nginx/safustaking.conf).

/** nginx's default 60 s, plus margin. */
export const QUICK_MS = 70_000;
/** Routes that scan or look a transaction up on chain (/claim, /covered-wallets/confirm): nginx 120 s. */
export const SCAN_MS = 130_000;
/** /claim/check scans every transaction, then the accepted set once more: nginx 180 s. */
export const CHECK_MS = 190_000;

/** A reason code the friendly-error tables translate (shell/lib/friendly-error.ts). */
export const REQUEST_TIMEOUT = "REQUEST_TIMEOUT";

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

export async function postJson<T>(path: string, body: unknown, timeoutMs = QUICK_MS): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(timeoutMs),
    });
  } catch (e) {
    if (e instanceof DOMException && e.name === "TimeoutError") throw new ApiError(0, REQUEST_TIMEOUT);
    throw e;
  }
  const out = await res.json().catch(() => ({}));
  if (!res.ok) throw new ApiError(res.status, typeof out.detail === "string" ? out.detail : `Request failed (${res.status})`);
  return out as T;
}
