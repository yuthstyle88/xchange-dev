// #539 DK-C: `desk-store.ts` — the state every step shares (design docs/design/xchange-v3-wave3-desk.md §5.3, D-5, §10
// row DK-C; quote contract docs/design/blueprint/quote-transaction.md §4.2/§4.3).
// Written before the implementation by the test-writer (claude/709ec749, on the FE lane controller claude/5edf7bb5's
// order; first part by claude/b060e9 at 8dedd84c). Do not edit, delete or weaken these tests; add your own elsewhere.
//
// [required] by these tests (desk-store.ts is reached through import.meta.glob, so a missing file fails an assertion):
// - `createDeskStore(deps: { api: ApiClient; path: DeskPath; me: Accessor<unknown> }): DeskStore` with the §5.3 names.
// - The config view (`path.configView`) is read once, on create (§3.1 L3: no polling).
// - `POST /api/quotes` (the command seam: `Idempotency-Key`) with `{ channel: "DESK", lines: [{ currency, shopSide,
//   quantity, feeThb? }] }`: `shopSide` is the desk's `type` (BUY/SELL, the shop's side as v2's form `side`),
//   `quantity` the amount exactly as typed (a string, D-4/W7), `feeThb` the fee as typed when one is set. No branch in
//   the body (§4.2: the branch is the seam's).
// - D-5: a change of type, currency, amount or fee re-quotes, debounced to one call per 300 ms; an older answer that
//   arrives after a newer one is dropped; `requote()` asks at once.
// - A quote refusal (4xx `{error:{code,field,message}}`) sets `quoteError` with the server's field and message, and no
//   quote.
// - `backToEdit()` clears `staffDraftId` and goes to step 0; `startNew()` keeps `staff` and `valueDate`.
// - `store.api` is the `ApiClient` given to `createDeskStore` (lane controller's ruling 1 on #539): steps reach the
//   server through it; `DeskStep.component` keeps `{ store }`.
//
// Types: the quote answer and the quote request are the generated `QuoteDto` (`shared/api/generated/quote.ts`) and
// `QuotesCommandInput` (`quotes.ts`), on main since TX-3/TX-4 (#669, #672).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { ApiClient } from "../../shared/api/client";
import {
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
  type QuoteDto,
  type QuotesCommandInput,
} from "../../shared/testing/fixtures";

type Store = Record<string, any>;
type StoreModule = { createDeskStore: (deps: { api: ApiClient; path: unknown; me: () => unknown }) => Store };
type PathModule = { OWNER_DESK: Record<string, any>; STAFF_DESK: Record<string, any> };
const storeMods = import.meta.glob<StoreModule>("./desk-store.ts");
const pathMods = import.meta.glob<PathModule>("./desk-path.ts");

async function load<T>(mods: Record<string, () => Promise<T>>, key: string): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/desk/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

const ME = () => ({ userId: "u-1", username: "ploy", name: "Ploy", roles: [], branchId: "MAIN", branchName: "XC-B", branches: [] });
const CONFIG = { shiftOpen: true };

let disposers: (() => void)[] = [];
async function makeStore(server: ReturnType<typeof fakeServer>, which: "OWNER_DESK" | "STAFF_DESK" = "OWNER_DESK") {
  const { createDeskStore } = await load(storeMods, "./desk-store.ts");
  const paths = await load(pathMods, "./desk-path.ts");
  const path = paths[which];
  let store!: Store;
  createRoot((dispose) => {
    disposers.push(dispose);
    store = createDeskStore({ api: server.api, path, me: ME });
  });
  return { store, path };
}

afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  vi.useRealTimers();
});

// ── the fake server ───────────────────────────────────────────────────────────────────────


type Call = {
  method: string;
  path: string;
  key: string | null;
  body: string;
  at: number;
};

type Answer = { status: number; body?: unknown; raw?: string; throws?: boolean };
type Route = (call: Call, nth: number) => Answer | Promise<Answer>;

/** A server answering by `METHOD path` (path without the query). Unrouted requests answer 404. */
function fakeServer() {
  const calls: Call[] = [];
  const routes = new Map<string, Route>();
  const counts = new Map<string, number>();
  let clock = 0;
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const method = String(init?.method ?? "GET").toUpperCase();
    const path = String(input).split("?")[0];
    const headers = new Headers(init?.headers);
    const body = init?.body === undefined || init?.body === null ? "" : typeof init.body === "string" ? init.body : "[binary]";
    const call: Call = { method, path, key: headers.get("idempotency-key"), body, at: clock++ };
    calls.push(call);
    const k = `${method} ${path}`;
    const n = counts.get(k) ?? 0;
    counts.set(k, n + 1);
    const route = routes.get(k);
    const a: Answer = route ? await route(call, n) : { status: 404, body: { error: { code: "notFound", message: "nf" } } };
    if (a.throws) throw new TypeError("Failed to fetch");
    const text = a.raw ?? JSON.stringify(a.body ?? {});
    return new Response(text, {
      status: a.status,
      headers: { "content-type": a.raw ? "text/html" : "application/json" },
    });
  });
  return {
    api: new ApiClient(fetchFn as unknown as typeof fetch),
    fetchFn,
    calls,
    on(method: string, path: string, route: Route) {
      routes.set(`${method.toUpperCase()} ${path}`, route);
    },
    callsTo(method: string, path: string): Call[] {
      return calls.filter((c) => c.method === method.toUpperCase() && c.path === path);
    },
  };
}

const view = (data: unknown): Answer => ({
  status: 200,
  body: { version: 1, changed: true, data },
});


function quoteDto(over: Partial<QuoteDto> & { quantity?: string } = {}) {
  const q = over.quantity ?? "100";
  return makeQuoteDto({
    id: over.id ?? "q-1",
    type: over.type ?? "BUY",
    channel: "DESK",
    expiresAt: over.expiresAt ?? new Date(Date.now() + 300_000).toISOString(),
    publication: makeQuotePublicationDto({ id: "pub-1", version: 3 }),
    lines: over.lines ?? [
      makeQuoteLineDto({
        currency: "USD",
        shopSide: "BUY",
        quantity: q,
        unitSize: "1",
        baseRate: "32.10",
        finalRate: "32.10",
        thbAmount: "3210.00",
        feeThb: "0.00",
        settleThb: "3210.00",
        publishedRateLineId: "line-1",
      }),
    ],
    benefits: [],
    stale: false,
  });
}


const refusal = (status: number, code: string, message = "refused", field?: string): Answer => ({
  status,
  body: { error: { code, message, ...(field ? { field } : {}) } },
});

async function flush(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

// ── the tests ───────────────────────────────────────────────────────────────────────────────

function serverWithQuotes(answerFor?: (call: Call, n: number) => Answer | Promise<Answer>) {
  const s = fakeServer();
  for (const v of ["desk_config", "staff_desk_config"]) s.on("GET", `/api/v3/views/${v}`, () => view(CONFIG));
  s.on("POST", "/api/quotes", answerFor ?? ((c) => ({ status: 200, body: quoteDto({ quantity: JSON.parse(c.body).lines[0].quantity }) })));
  return s;
}

for (const which of ["OWNER_DESK", "STAFF_DESK"] as const) {
  describe(`DeskStore on ${which}`, () => {
    beforeEach(() => vi.useFakeTimers());

    it("S1 reads its config view once, on create, and never again (no polling)", async () => {
      const s = serverWithQuotes();
      const { store, path } = await makeStore(s, which);
      await vi.waitFor(() => expect(store.config()).toEqual(CONFIG));
      store.setAmount("100");
      await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
      expect(s.callsTo("GET", `/api/v3/views/${path.configView}`)).toHaveLength(1);
    });

    it("S2 three amount edits inside 300 ms ask for one quote, with the last amount", async () => {
      const s = serverWithQuotes();
      const { store } = await makeStore(s, which);
      store.setCurrency("USD");
      store.setType("BUY");
      store.setAmount("1");
      await vi.advanceTimersByTimeAsync(100);
      store.setAmount("10");
      await vi.advanceTimersByTimeAsync(100);
      store.setAmount("100");
      await vi.advanceTimersByTimeAsync(299);
      expect(s.callsTo("POST", "/api/quotes")).toHaveLength(0);
      await vi.advanceTimersByTimeAsync(2);
      await vi.waitFor(() => expect(s.callsTo("POST", "/api/quotes")).toHaveLength(1));
      expect(JSON.parse(s.callsTo("POST", "/api/quotes")[0].body).lines[0].quantity).toBe("100");
    });

    it("S3 the quote request is the command seam's: a key, channel DESK, one line, money as typed, no branch value", async () => {
      const s = serverWithQuotes();
      const { store } = await makeStore(s, which);
      store.setType("SELL");
      store.setCurrency("JPY");
      store.setAmount("10000.50");
      await store.requote();
      const call = s.callsTo("POST", "/api/quotes")[0];
      expect(call.key, "an Idempotency-Key").toBeTruthy();
      const body = JSON.parse(call.body);
      expect(body.channel).toBe("DESK");
      expect(body.lines).toEqual([expect.objectContaining({ currency: "JPY", shopSide: "SELL", quantity: "10000.50" })]);
      // the branch is the seam's (§4.2): the generated input carries `branch`/`branchId` as nullable, so they may be
      // absent or null, never a value
      expect(body.branchId ?? null, "no branchId value").toBeNull();
      expect(body.branch ?? null, "no branch value").toBeNull();
      const known: (keyof QuotesCommandInput)[] = ["branch", "branchId", "customerId", "channel", "lines"];
      for (const k of Object.keys(body)) expect(known, `quote body key ${k}`).toContain(k);
    });

    it("S4 a fee typed at step 3 re-quotes and goes as the line's feeThb, as typed", async () => {
      const s = serverWithQuotes();
      const { store } = await makeStore(s, which);
      store.setCurrency("USD");
      store.setAmount("100");
      await store.requote();
      store.setFee("50.25");
      await vi.advanceTimersByTimeAsync(301);
      await vi.waitFor(() => expect(s.callsTo("POST", "/api/quotes").length).toBeGreaterThanOrEqual(2));
      const last = s.callsTo("POST", "/api/quotes").at(-1)!;
      expect(JSON.parse(last.body).lines[0].feeThb).toBe("50.25");
    });

    it("S5 type and currency changes re-quote too (D-5)", async () => {
      const s = serverWithQuotes();
      const { store } = await makeStore(s, which);
      store.setAmount("100");
      store.setCurrency("USD");
      await vi.advanceTimersByTimeAsync(301);
      await vi.waitFor(() => expect(s.callsTo("POST", "/api/quotes")).toHaveLength(1));
      store.setType("SELL");
      await vi.advanceTimersByTimeAsync(301);
      await vi.waitFor(() => expect(s.callsTo("POST", "/api/quotes")).toHaveLength(2));
      store.setCurrency("EUR");
      await vi.advanceTimersByTimeAsync(301);
      await vi.waitFor(() => expect(s.callsTo("POST", "/api/quotes")).toHaveLength(3));
    });

    it("S6 an older quote answer that arrives after a newer one is dropped", async () => {
      let releaseFirst!: () => void;
      const firstHeld = new Promise<void>((r) => (releaseFirst = r));
      const s = serverWithQuotes(async (c, n) => {
        if (n === 0) await firstHeld;
        return { status: 200, body: quoteDto({ id: `q-${n}`, quantity: JSON.parse(c.body).lines[0].quantity }) };
      });
      const { store } = await makeStore(s, which);
      store.setCurrency("USD");
      store.setAmount("1");
      const first = store.requote();
      store.setAmount("2");
      await store.requote();
      expect(store.quote()?.id).toBe("q-1");
      releaseFirst();
      await first;
      await flush();
      expect(store.quote()?.id, "the older answer does not replace the newer").toBe("q-1");
    });

    it("S7 the quote is held as the server answered it: figures unchanged (D-4)", async () => {
      const s = serverWithQuotes(() => ({ status: 200, body: quoteDto({ id: "q-x" }) }));
      const { store } = await makeStore(s, which);
      store.setCurrency("USD");
      store.setAmount("100");
      await store.requote();
      expect(store.quote()).toEqual(quoteDto({ id: "q-x", expiresAt: store.quote()?.expiresAt }));
    });

    it("S8 a quote refusal sets quoteError with the server's field and message, and no quote", async () => {
      const s = serverWithQuotes(() => refusal(400, "validationFailed", "จำนวนเงินไม่ถูกต้อง", "amount"));
      const { store } = await makeStore(s, which);
      store.setCurrency("USD");
      store.setAmount("abc");
      await store.requote();
      expect(store.quote()).toBeUndefined();
      expect(store.quoteError()).toEqual({ field: "amount", message: "จำนวนเงินไม่ถูกต้อง" });
    });

    it("S9 backToEdit clears staffDraftId and goes back to step 0", async () => {
      const s = serverWithQuotes();
      const { store } = await makeStore(s, which);
      store.setStaffDraftId("draft-1");
      store.go(1);
      store.backToEdit();
      expect(store.staffDraftId()).toBe("");
      expect(store.step()).toBe(0);
    });

    it("S10 startNew keeps staff and valueDate, clears the exchange and staffDraftId", async () => {
      const s = serverWithQuotes();
      const { store } = await makeStore(s, which);
      store.setValueDate("2026-10-11");
      store.setCurrency("USD");
      store.setAmount("100");
      store.setStaffDraftId("draft-1");
      const staff = store.staff();
      vi.spyOn(window, "confirm").mockReturnValue(true);
      store.startNew();
      expect(store.valueDate()).toBe("2026-10-11");
      expect(store.staff()).toBe(staff);
      expect(store.amount()).toBe("");
      expect(store.staffDraftId()).toBe("");
      expect(store.step()).toBe(0);
    });

    it("S11 a user edit of a passport field clears identityChecked; an OCR fill does not", async () => {
      const s = serverWithQuotes();
      const { store } = await makeStore(s, which);
      store.setIdentityChecked(true);
      store.setField("name", "JOHN", "ocr");
      expect(store.identityChecked()).toBe(true);
      store.setField("name", "JON", "user");
      expect(store.identityChecked()).toBe(false);
      expect([...store.editedByUser()]).toContain("name");
    });

    it("S13 the store carries the ApiClient it was given, for the steps (lane controller's ruling 1 on #539)", async () => {
      const s = serverWithQuotes();
      const { store } = await makeStore(s, which);
      expect(store.api).toBe(s.api);
    });

    it("S12 any fee edit clears paymentChecked (dh:117)", async () => {
      const s = serverWithQuotes();
      const { store } = await makeStore(s, which);
      store.setPaymentChecked(true);
      store.setFee("10");
      expect(store.paymentChecked()).toBe(false);
    });
  });
}
