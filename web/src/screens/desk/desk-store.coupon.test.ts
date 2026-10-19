// B03 desk coupon on the quote (FE-desk, #564; design xchange-v3-wave3-desk.md D-5 "a new quote on each change of …
// benefits", marketing-campaigns.md D-9, §10 "`POST /api/quotes` gains optional `couponCode`", errors `couponInvalid`).
// Written before the implementation by the FE-desk sub-controller claude/dell-claude-1. Do not edit, delete or weaken
// these tests; add your own in another file.
//
// [required] by these tests (store members `couponCode` / `setCouponCode` exist since the skeleton):
// - `setCouponCode(c)` re-quotes like a change of type, currency or amount (debounced 300 ms, D-5).
// - The quote body carries `couponCode` trimmed when it is not blank, and has no `couponCode` key when it is blank.
// - A refusal with code `couponInvalid` sets `quoteError` with `field: "couponCode"` (the server sends no field), and no
//   quote. Other refusals keep today's mapping (`desk-store.test.ts`).
// - `startNew()` clears the coupon; `backToEdit()` keeps it, and its re-quote carries it.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { ApiClient } from "../../shared/api/client";
import { createDeskStore } from "./desk-store";
import { OWNER_DESK, STAFF_DESK } from "./desk-path";

type Call = { path: string; body: string };
type Answer = { status: number; body?: unknown };

function fakeServer(quote: (body: any, n: number) => Answer) {
  const calls: Call[] = [];
  let n = 0;
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input).split("?")[0];
    const body = typeof init?.body === "string" ? init.body : "";
    calls.push({ path, body });
    let a: Answer;
    if (path.startsWith("/api/v3/views/")) a = { status: 200, body: { version: 1, changed: true, data: { shiftOpen: true, rates: [] } } };
    else if (path === "/api/quotes") a = quote(JSON.parse(body), n++);
    else a = { status: 404, body: { error: { code: "notFound", message: "nf" } } };
    return new Response(JSON.stringify(a.body ?? {}), { status: a.status, headers: { "content-type": "application/json" } });
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), quotes: () => calls.filter((c) => c.path === "/api/quotes").map((c) => JSON.parse(c.body)) };
}

function quoteDto(id = "q-1") {
  return {
    id,
    type: "BUY",
    channel: "DESK",
    expiresAt: new Date(Date.now() + 300_000).toISOString(),
    publication: { id: "p", version: 1 },
    lines: [{ currency: "USD", shopSide: "BUY", quantity: "100", unitSize: "1", baseRate: "32.10", finalRate: "32.20", thbAmount: "3220.00", feeThb: "0.00", settleThb: "3220.00", publishedRateLineId: "l" }],
    benefits: [],
    stale: false,
  };
}

const ME = () => ({ userId: "u-1", name: "Ploy", roles: [], branchId: "MAIN", branchName: "XC-B", branches: [] });
let disposers: (() => void)[] = [];
function makeStore(api: ApiClient, path: typeof OWNER_DESK) {
  let store!: ReturnType<typeof createDeskStore>;
  createRoot((d) => {
    disposers.push(d);
    store = createDeskStore({ api, path, me: ME as never });
  });
  return store;
}
async function settle() {
  await vi.advanceTimersByTimeAsync(400);
  for (let i = 0; i < 6; i++) await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(window, "confirm").mockReturnValue(true);
});
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  vi.useRealTimers();
  vi.restoreAllMocks();
});

for (const [label, path] of [["OWNER_DESK", OWNER_DESK], ["STAFF_DESK", STAFF_DESK]] as const) {
  describe(`desk coupon on ${label}`, () => {
    async function ready(server: ReturnType<typeof fakeServer>) {
      const store = makeStore(server.api, path);
      store.setCurrency("USD");
      store.setAmount("100");
      await settle();
      return store;
    }

    it("C1 setCouponCode re-quotes once, debounced, with the code trimmed", async () => {
      const server = fakeServer(() => ({ status: 200, body: quoteDto() }));
      const store = await ready(server);
      const before = server.quotes().length;
      store.setCouponCode("  SAVE10 ");
      await settle();
      const after = server.quotes();
      expect(after.length).toBe(before + 1);
      expect(after.at(-1).couponCode).toBe("SAVE10");
    });

    it("C2 a blank coupon sends no couponCode key", async () => {
      const server = fakeServer(() => ({ status: 200, body: quoteDto() }));
      const store = await ready(server);
      store.setCouponCode("SAVE10");
      await settle();
      store.setCouponCode("   ");
      await settle();
      expect("couponCode" in server.quotes().at(-1)).toBe(false);
    });

    it("C3 a couponInvalid refusal sets quoteError on the couponCode field, and no quote", async () => {
      const server = fakeServer((body) =>
        body.couponCode ? { status: 400, body: { error: { code: "couponInvalid", message: "couponInvalid" } } } : { status: 200, body: quoteDto() },
      );
      const store = await ready(server);
      expect(store.quote()).toBeDefined();
      store.setCouponCode("NOPE");
      await settle();
      expect(store.quote()).toBeUndefined();
      expect(store.quoteError()?.field).toBe("couponCode");
      expect(store.quoteError()?.message ?? "").not.toBe("");
    });

    it("C4 startNew clears the coupon", async () => {
      const server = fakeServer(() => ({ status: 200, body: quoteDto() }));
      const store = await ready(server);
      store.setCouponCode("SAVE10");
      await settle();
      expect(store.startNew()).toBe(true);
      expect(store.couponCode()).toBe("");
    });

    it("C5 backToEdit keeps the coupon, and its re-quote carries it", async () => {
      const server = fakeServer(() => ({ status: 200, body: quoteDto() }));
      const store = await ready(server);
      store.setCouponCode("SAVE10");
      await settle();
      store.backToEdit();
      await settle();
      expect(store.couponCode()).toBe("SAVE10");
      expect(server.quotes().at(-1).couponCode).toBe("SAVE10");
    });
  });
}
