import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, REQUEST_TIMEOUT, postJson } from "./http";
import { makeFriendlyError } from "./friendly-error";

const friendly = makeFriendlyError({ own: [], hints: {}, patterns: [] });

describe("postJson", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("turns a timeout into REQUEST_TIMEOUT, which every pool's errors translate", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new DOMException("timed out", "TimeoutError")));
    const err = await postJson<never>("/claim", {}, 10).catch((e: Error) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err.message).toBe(REQUEST_TIMEOUT);
    expect(friendly(err).message).toMatch(/took too long to answer/);
  });

  it("passes the backend's reason through, and a non-text detail becomes the status line", async () => {
    vi.stubGlobal("fetch", () => Promise.resolve(new Response(JSON.stringify({ detail: "WALLET_NOT_COVERED" }), { status: 400 })));
    expect((await postJson<never>("/claim", {}).catch((e: Error) => e)).message).toBe("WALLET_NOT_COVERED");
    vi.stubGlobal("fetch", () => Promise.resolve(new Response(JSON.stringify({ detail: [{ loc: ["x"] }] }), { status: 422 })));
    expect((await postJson<never>("/claim", {}).catch((e: Error) => e)).message).toBe("Request failed (422)");
  });
});
