// #539 DK-C: the save (`DeskStore.submit`, `save.ts`) — design docs/design/xchange-v3-wave3-desk.md §5.4, D-10, §10 row
// DK-C, and the call order of docs/design/blueprint/quote-transaction.md §4.2 and §10 ("Desk": POST /api/quotes, POST
// /api/transactions, POST /api/transactions/:id/confirm, then complete when CONFIRMED; PENDING_APPROVAL re-reads GET
// /api/v3/views/transaction?id=, no new submit, no new key).
// Written before the implementation by the test-writer (claude/709ec749, on the FE lane controller claude/5edf7bb5's
// order). Do not edit, delete or weaken these tests; add your own elsewhere.
//
// [required] by these tests:
// - submit(): uploads the passport (and the slip when set) to `path.upload` first (answer `{uploadKey}`); refuses to go
//   on with an expired quote and asks for a new one first; then the three commands above, each with an
//   `Idempotency-Key`. The create body is §4.2's: `quoteId`, `customerId`, `passport: {uploadKey}`, `payment: {method,
//   reference, account, slip?, bankFeeThb, paidAt}`, `valueDate`, `notes`, `countConfirmed`, and the optional
//   `staffDraftId`, `bookingId`, `kioskRequestId`. No figure the quote did not give.
// - The outcome follows the server's transaction state (§06, verbatim in `transactionState`): COMPLETED → `saved`;
//   PENDING_APPROVAL → neither saved nor refused, no `complete`, the view is re-read; CANCELLED → not saved, the next
//   submit is a new exchange; a 4xx with an error code → `refused` with the server's message (`quoteExpired` asks for a
//   new quote); a 5xx, a non-JSON body, a 2xx without a state, or a network failure → `unknown`.
// - D-10: a retry after `unknown` resends the failed request byte for byte with the same key, and does not redo the
//   steps that succeeded; `backToEdit()` drops the keys, so the next submit is a new exchange.
// - W7: no money arithmetic in the folder's sources (a source scan).
//
// - The upload goes through `ApiClient.postBytes` (lane controller's ruling on #539, gap 3): the file itself as the
//   body, its own type as the Content-Type, an Idempotency-Key (`api/src/presentation/http/files.rs:140`).
//
// Not pinned here, on purpose: the shape of the transaction answer beyond `{id, status}` (see txAnswer; the ruling
// sends it to #672).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { ApiClient } from "../../shared/api/client";
import {
  makeQuoteDto,
  makeQuoteLineDto,
  makeQuotePublicationDto,
  makeTransactionDto,
  makeTransactionLineDto,
  makeTransactionPaymentDto,
  type QuoteDto,
  type TransactionCreateInput,
  type TransactionDto,
} from "../../shared/testing/fixtures";

type Store = Record<string, any>;
type StoreModule = { createDeskStore: (deps: { api: ApiClient; path: unknown; me: () => unknown }) => Store };
type PathModule = { OWNER_DESK: Record<string, any>; STAFF_DESK: Record<string, any> };
const storeMods = import.meta.glob<StoreModule>("./desk-store.ts");
const pathMods = import.meta.glob<PathModule>("./desk-path.ts");
const SOURCES = import.meta.glob<string>(["./*.ts", "./*.tsx", "!./*.test.ts", "!./*.test.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
});

async function load<T>(mods: Record<string, () => Promise<T>>, key: string): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/desk/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

const ME = () => ({ userId: "u-1", username: "ploy", name: "Ploy", roles: [], branchId: "MAIN", branchName: "XC-B", branches: [] });
const PASSPORT = new File([new Uint8Array([1, 2, 3])], "passport.jpg", { type: "image/jpeg" });

let disposers: (() => void)[] = [];
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
  contentType: string | null;
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
    const call: Call = { method, path, key: headers.get("idempotency-key"), contentType: headers.get("content-type"), body, at: clock++ };
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

/** The answer of `POST /api/transactions` and `…/confirm`: the generated `TransactionDto` (TX-4 #672,
 *  `shared/api/generated/transactions.ts`), answered at the top level as the command seam answers every `Output`
 *  (`command.rs:440`, `serde_json::to_value(&output)`). `…/complete` is TX-5's (#873, not merged): its answer is design
 *  §4.2's "TransactionDto COMPLETED + ReceiptDto"; until #873 generates it, the fake answers the same `TransactionDto`
 *  with `status: "COMPLETED"` and its `documentRef`, and the tests read only `id` and `status` from it. */
function txDto(id: string, status: string, extra: Partial<TransactionDto> = {}) {
  return makeTransactionDto({
    id,
    branchId: "MAIN",
    mode: "LIVE",
    status,
    documentRef: null,
    quoteId: "q-1",
    customerId: "c-1",
    approvalRequestId: null,
    cancelReason: null,
    cancelledAt: null,
    recordedAt: "2026-10-11T03:00:00Z",
    lines: [
      makeTransactionLineDto({
        currency: "USD",
        shopSide: "BUY",
        quantity: "100",
        unitSize: "1",
        rateSnapshot: "35.12000000",
        thbAmount: "3512.00",
        feeThb: "0.00",
        settleThb: "3512.00",
      }),
    ],
    payment: makeTransactionPaymentDto({ method: "Cash", reference: "", account: "", amountThb: "3512.00", bankFeeThb: "0.00", paidAt: "2026-10-11T03:00:00Z" }),
    ...extra,
  });
}

function txAnswer(id: string, status: string, extra: Partial<TransactionDto> = {}): Answer {
  return { status: 200, body: txDto(id, status, extra) };
}

const refusal = (status: number, code: string, message = "refused", field?: string): Answer => ({
  status,
  body: { error: { code, message, ...(field ? { field } : {}) } },
});


// ── a ready exchange ──────────────────────────────────────────────────────────────────────

type Flow = { create?: Answer[]; confirm?: Answer[]; complete?: Answer[]; quoteExpiresAt?: string };

function serverFor(flow: Flow = {}) {
  const s = fakeServer();
  for (const v of ["desk_config", "staff_desk_config"]) s.on("GET", `/api/v3/views/${v}`, () => view({ shiftOpen: true }));
  let quotes = 0;
  s.on("POST", "/api/quotes", () => {
    quotes += 1;
    return { status: 200, body: quoteDto({ id: `q-${quotes}`, expiresAt: quotes === 1 && flow.quoteExpiresAt ? flow.quoteExpiresAt : undefined }) };
  });
  for (const up of ["/api/upload", "/api/staff/upload"]) {
    let n = 0;
    s.on("POST", up, () => ({ status: 200, body: { uploadKey: `key-${++n}` } }));
  }
  const seq = (list: Answer[] | undefined, dflt: (n: number) => Answer) => (_c: Call, n: number) => (list && list[n]) ?? dflt(n);
  s.on("POST", "/api/transactions", seq(flow.create, () => txAnswer("tx-1", "DRAFT")));
  s.on("POST", "/api/transactions/tx-1/confirm", seq(flow.confirm, () => txAnswer("tx-1", "CONFIRMED")));
  s.on("POST", "/api/transactions/tx-1/complete", seq(flow.complete, () => txAnswer("tx-1", "COMPLETED", { documentRef: "XC-20261011-000001" })));
  s.on("GET", "/api/v3/views/transaction", () => view({ id: "tx-1", status: "PENDING_APPROVAL" }));
  return s;
}

async function ready(s: ReturnType<typeof fakeServer>, which: "OWNER_DESK" | "STAFF_DESK") {
  const { createDeskStore } = await load(storeMods, "./desk-store.ts");
  const paths = await load(pathMods, "./desk-path.ts");
  const path = paths[which];
  let store!: Store;
  createRoot((dispose) => {
    disposers.push(dispose);
    store = createDeskStore({ api: s.api, path, me: ME });
  });
  await vi.waitFor(() => expect(store.config()).toBeDefined());
  store.setType("BUY");
  store.setCurrency("USD");
  store.setAmount("100");
  await store.requote();
  store.setPassport(PASSPORT);
  for (const [k, v] of Object.entries({ name: "JOHN DOE", passportNo: "AA1234567", nationality: "USA", birthDate: "1990-01-01", passportExpiry: "2030-01-01" })) {
    store.setField(k, v, "user");
  }
  store.setIdentityChecked(true);
  store.setCustomerId("CX-00000001");
  store.setValueDate("2026-10-11");
  store.setMethod("Cash");
  store.setAccount("Cash drawer 1");
  store.setBankFee("0");
  store.setNotes("");
  store.setPaymentChecked(true);
  return { store, path };
}

const FIGURES = ["amount", "rate", "thbAmount", "settleThb", "finalRate", "baseRate", "unitSize", "principal", "fee", "quantity", "total"];
/** §4.2's create body: exactly the keys of the generated `TransactionCreateInput` (the compiler checks the list). */
const CREATE_KEYS: (keyof TransactionCreateInput)[] = ["quoteId", "customerId", "bookingId", "kioskRequestId", "staffDraftId", "passport", "payment", "valueDate", "notes", "countConfirmed"];

for (const which of ["OWNER_DESK", "STAFF_DESK"] as const) {
  describe(`submit on ${which}`, () => {
    beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));

    it("V1 uploads first, then create, confirm and complete in that order, each with a key; COMPLETED is saved", async () => {
      const s = serverFor();
      const { store, path } = await ready(s, which);
      await expect(store.submit()).resolves.toBe("saved");
      const order = s.calls.filter((c) => c.method === "POST" && c.path !== "/api/quotes").map((c) => c.path);
      expect(order).toEqual([path.upload, "/api/transactions", "/api/transactions/tx-1/confirm", "/api/transactions/tx-1/complete"]);
      for (const p of ["/api/transactions", "/api/transactions/tx-1/confirm", "/api/transactions/tx-1/complete"]) {
        expect(s.callsTo("POST", p)[0].key, `${p} carries an Idempotency-Key`).toBeTruthy();
      }
      expect(store.outcome()).toBe("saved");
      expect(store.transactionState()).toBe("COMPLETED");
    });

    it("V1b the passport is uploaded as itself: raw bytes, the file's own Content-Type, an Idempotency-Key (postBytes)", async () => {
      const s = serverFor();
      const { store, path } = await ready(s, which);
      await store.submit();
      const up = s.callsTo("POST", path.upload)[0];
      expect(up, "one upload").toBeDefined();
      expect(up.contentType).toBe("image/jpeg");
      expect(up.key, "an Idempotency-Key on the upload").toBeTruthy();
      expect(up.body, "not a JSON body").toBe("[binary]");
    });

    it("V2 the create body is §4.2's: the quote id, the uploaded key, the payment object; no figure", async () => {
      const s = serverFor();
      const { store } = await ready(s, which);
      await store.submit();
      const body = JSON.parse(s.callsTo("POST", "/api/transactions")[0].body);
      expect(body.quoteId).toBe(store.quote()?.id ?? "q-1");
      expect(body.customerId).toBe("CX-00000001");
      expect(body.passport).toEqual({ uploadKey: "key-1" });
      expect(body.valueDate).toBe("2026-10-11");
      expect(body.payment).toEqual(expect.objectContaining({ method: "Cash", account: "Cash drawer 1", bankFeeThb: "0" }));
      for (const k of Object.keys(body)) expect(CREATE_KEYS, `create body key ${k}`).toContain(k);
      for (const f of FIGURES) expect(body, `no figure ${f} in the create body`).not.toHaveProperty(f);
    });

    it("V3 an expired quote is not used: a new quote is asked first, and create names the new one", async () => {
      const s = serverFor({ quoteExpiresAt: new Date(Date.now() - 1000).toISOString() });
      const { store } = await ready(s, which);
      await store.submit();
      const quotes = s.callsTo("POST", "/api/quotes");
      const create = s.callsTo("POST", "/api/transactions")[0];
      expect(quotes.length).toBeGreaterThanOrEqual(2);
      expect(quotes.at(-1)!.at).toBeLessThan(create.at);
      expect(JSON.parse(create.body).quoteId).toBe("q-2");
    });

    it("V4 PENDING_APPROVAL is waiting: neither saved nor refused, no complete, and the transaction view is re-read", async () => {
      const s = serverFor({ confirm: [txAnswer("tx-1", "PENDING_APPROVAL", { approvalRequestId: "ar-1" })] });
      const { store } = await ready(s, which);
      const out = await store.submit();
      expect(["saved", "refused"]).not.toContain(out);
      expect(store.transactionState()).toBe("PENDING_APPROVAL");
      expect(s.callsTo("POST", "/api/transactions/tx-1/complete")).toHaveLength(0);
      await vi.advanceTimersByTimeAsync(60_000);
      await vi.waitFor(() => expect(s.callsTo("GET", "/api/v3/views/transaction").length).toBeGreaterThanOrEqual(1));
      expect(s.calls.some((c) => c.method === "GET" && c.path === "/api/v3/views/transaction")).toBe(true);
      expect(s.callsTo("POST", "/api/transactions"), "no new submit").toHaveLength(1);
    });

    it("V5 CANCELLED is shown as cancelled, not saved; the next submit is a new exchange with a new key", async () => {
      const s = serverFor({ confirm: [txAnswer("tx-1", "CANCELLED", { cancelReason: "rejected" })] });
      const { store } = await ready(s, which);
      const out = await store.submit();
      expect(out).not.toBe("saved");
      expect(store.transactionState()).toBe("CANCELLED");
      await store.requote();
      await store.submit();
      const creates = s.callsTo("POST", "/api/transactions");
      expect(creates).toHaveLength(2);
      expect(creates[1].key).not.toBe(creates[0].key);
    });

    for (const [n, step] of [["a", "create"], ["b", "confirm"], ["c", "complete"]] as const) {
      it(`V6${n} a 4xx with an error code at ${step} is refused, with the server's message`, async () => {
        const s = serverFor({ [step]: [refusal(400, "validationFailed", "ข้อมูลไม่ครบ")] } as Flow);
        const { store } = await ready(s, which);
        await expect(store.submit()).resolves.toBe("refused");
        expect(store.outcome()).toBe("refused");
        expect(store.saveError()).toBe("ข้อมูลไม่ครบ");
      });
    }

    it("V7 quoteExpired is refused and a new quote is asked for", async () => {
      const s = serverFor({ confirm: [refusal(409, "quoteExpired", "ราคาหมดอายุ")] });
      const { store } = await ready(s, which);
      // let the setters' own debounced re-quote settle first, so only the refusal can ask for the next one
      await vi.advanceTimersByTimeAsync(400);
      await vi.waitFor(() => expect(store.quoting()).toBe(false));
      const before = s.callsTo("POST", "/api/quotes").length;
      await expect(store.submit()).resolves.toBe("refused");
      await vi.advanceTimersByTimeAsync(301);
      await vi.waitFor(() => expect(s.callsTo("POST", "/api/quotes").length).toBeGreaterThan(before));
    });

    const unknowns: [string, Answer][] = [
      ["a 5xx", { status: 503, body: { error: { code: "busy", message: "busy" } } }],
      ["a body that is not JSON", { status: 200, raw: "<html>proxy</html>" }],
      ["a 2xx without a transaction state", { status: 200, body: {} }],
      ["a network failure", { status: 0, throws: true }],
    ];
    for (const [i, [what, answer]] of unknowns.entries()) {
      it(`V8${"abcd"[i]} ${what} at complete is unknown, not refused and not saved`, async () => {
        const s = serverFor({ complete: [answer] });
        const { store } = await ready(s, which);
        await expect(store.submit()).resolves.toBe("unknown");
        expect(store.outcome()).toBe("unknown");
      });
    }

    it("V9 a retry after unknown resends the failed complete byte for byte with the same key, and redoes nothing else", async () => {
      const s = serverFor({ complete: [{ status: 503, body: { error: { code: "busy", message: "busy" } } }] });
      const { store } = await ready(s, which);
      await expect(store.submit()).resolves.toBe("unknown");
      await expect(store.submit()).resolves.toBe("saved");
      const completes = s.callsTo("POST", "/api/transactions/tx-1/complete");
      expect(completes).toHaveLength(2);
      expect(completes[1].key).toBe(completes[0].key);
      expect(completes[1].body).toBe(completes[0].body);
      expect(s.callsTo("POST", "/api/transactions"), "create not redone").toHaveLength(1);
      expect(s.callsTo("POST", "/api/transactions/tx-1/confirm"), "confirm not redone").toHaveLength(1);
    });

    it("V10 a retry after an unknown create resends the same create, same key, same body", async () => {
      const s = serverFor({ create: [{ status: 0, throws: true }] });
      const { store } = await ready(s, which);
      await expect(store.submit()).resolves.toBe("unknown");
      await store.submit();
      const creates = s.callsTo("POST", "/api/transactions");
      expect(creates).toHaveLength(2);
      expect(creates[1].key).toBe(creates[0].key);
      expect(creates[1].body).toBe(creates[0].body);
    });

    it("V11 backToEdit after unknown drops the keys: the next submit is a new exchange", async () => {
      const s = serverFor({ create: [{ status: 0, throws: true }] });
      const { store } = await ready(s, which);
      await store.submit();
      store.backToEdit();
      await store.requote();
      await store.submit();
      const creates = s.callsTo("POST", "/api/transactions");
      expect(creates[1].key).not.toBe(creates[0].key);
    });
  });
}

describe("W7: no money arithmetic in the desk's own sources", () => {
  it("M1 no Number(, parseFloat, parseInt, Math.round, toFixed or * 100 outside tests", () => {
    expect(Object.keys(SOURCES)).toContain("./desk-store.ts");
    expect(Object.keys(SOURCES)).toContain("./save.ts");
    for (const [file, src] of Object.entries(SOURCES)) {
      expect(src, file).not.toMatch(/Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100/);
    }
  });

  it("M2 no fetch( in the desk's own sources: every call goes through ApiClient", () => {
    for (const [file, src] of Object.entries(SOURCES)) {
      expect(src, file).not.toMatch(/\bfetch\(/);
    }
  });
});
