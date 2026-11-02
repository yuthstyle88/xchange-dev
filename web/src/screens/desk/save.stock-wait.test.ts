// FE-W #5978: complete waits for a drawer top-up (design xchange-v3-wave3-desk.md §5.4a, as amended by PR #5977 after the
// owner's "ต้องเติมเงินก่อน ห้ามติดลบ"; server ledger-inventory.md §5.1.2). Restated by claude/01TxaRJG (FE sub-controller)
// from #5932's tests: there is no NEGATIVE_STOCK approval any more, so the old S4/S4b (rejection) rows are removed and
// approvalRequired from complete is a refusal (S7). Do not edit, delete or weaken; add tests in a file of your own. The fake
// server and `ready()` are save.test.ts's, copied unchanged.
//
// §5.4a: `POST …/complete` answering `409 stockInsufficient` is a wait, not a refusal: the transaction and its complete
// key are kept, complete is resent every APPROVAL_POLL_MS with the same key, and `stockWait` = {message}, the server's
// message as sent. One test per row; the row id is in the test name. P1 is the unchanged PENDING_APPROVAL wait.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";
import { ApiClient } from "../../shared/api/client";
import {
  makeQuoteDto,
  makeTransactionDetailDto,
  makeTransactionViewData,
  makeQuoteLineDto,
  makeQuotePublicationDto,
  makeTransactionDto,
  makeTransactionLineDto,
  makeTransactionPaymentDto,
  type QuoteDto,
  type TransactionDto,
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
    cashSettlement: over.cashSettlement ?? false,
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
  s.on("POST", "/api/quotes", (call) => {
    quotes += 1;
    // As the server does (CR-b, quote.rs:479): a quote asked with paymentMethod "Cash" is cash-settled (#5229).
    const cashSettlement = (JSON.parse(call.body || "{}") as { paymentMethod?: string }).paymentMethod === "Cash";
    return {
      status: 200,
      body: quoteDto({ id: `q-${quotes}`, cashSettlement, expiresAt: quotes === 1 && flow.quoteExpiresAt ? flow.quoteExpiresAt : undefined }),
    };
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


const SHORT = "ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: THB ลิ้นชัก ขาด 1711.46";
const SHORT_LESS = "ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: THB ลิ้นชัก ขาด 211.46";
/** §5.1.2 "What the desk sees": the port's refusal, with no field. */
const shortAnswer = (message = SHORT) => refusal(409, "stockInsufficient", message);
/** `GET /api/v3/views/transaction?id=` in its real shape (generated `TransactionViewData = { transaction }`). */
const txView = (status: string, cancelReason: string | null = null) =>
  view(makeTransactionViewData({ transaction: makeTransactionDetailDto({ id: "tx-1", status, cancelReason }) }));

describe("§5.4a complete waits for a drawer top-up", () => {
  beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));

  it("S1 complete → 409 stockInsufficient: a wait, not a refusal; the transaction is kept and the message is kept as sent", async () => {
    const s = serverFor({ complete: [shortAnswer()] });
    const { store } = await ready(s, "STAFF_DESK");
    const out = await store.submit();
    expect(out).toBe("saving");
    expect(store.outcome()).toBe("saving");
    expect(store.transactionState()).toBe("CONFIRMED");
    expect(store.stockWait()).toEqual({ message: SHORT });
    expect(store.saveError() ?? "", "the code is not shown").not.toContain("stockInsufficient");
  });

  it("S2 while short, complete is resent every APPROVAL_POLL_MS with the same key; no new create; the message follows the latest answer", async () => {
    const s = serverFor({ complete: [shortAnswer(), shortAnswer(SHORT_LESS), shortAnswer(SHORT_LESS)] });
    const { store } = await ready(s, "STAFF_DESK");
    await store.submit();
    expect(s.callsTo("POST", "/api/transactions/tx-1/complete")).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(15_000);
    await vi.waitFor(() => expect(s.callsTo("POST", "/api/transactions/tx-1/complete")).toHaveLength(2));
    await vi.waitFor(() => expect(store.stockWait()).toEqual({ message: SHORT_LESS }));
    await vi.advanceTimersByTimeAsync(15_000);
    await vi.waitFor(() => expect(s.callsTo("POST", "/api/transactions/tx-1/complete")).toHaveLength(3));
    const keys = new Set(s.callsTo("POST", "/api/transactions/tx-1/complete").map((c) => c.key));
    expect(keys.size, "one key for every resend").toBe(1);
    expect(s.callsTo("POST", "/api/transactions"), "no new create").toHaveLength(1);
    expect(s.callsTo("POST", "/api/transactions/tx-1/confirm"), "no new confirm").toHaveLength(1);
    expect(s.callsTo("GET", "/api/v3/views/transaction"), "no transaction read").toHaveLength(0);
  });

  it("S3 topped up: the next complete answers COMPLETED → saved, stockWait cleared, the resend stops", async () => {
    const s = serverFor({ complete: [shortAnswer(), txAnswer("tx-1", "COMPLETED", { documentRef: "XC-20261102-000009" })] });
    const { store } = await ready(s, "STAFF_DESK");
    await store.submit();
    await vi.advanceTimersByTimeAsync(15_000);
    await vi.waitFor(() => expect(store.outcome()).toBe("saved"));
    expect(store.saved()?.documentRef).toBe("XC-20261102-000009");
    expect(store.stockWait()).toBeUndefined();
    const n = s.callsTo("POST", "/api/transactions/tx-1/complete").length;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(s.callsTo("POST", "/api/transactions/tx-1/complete"), "no complete after COMPLETED").toHaveLength(n);
  });

  it("S5 cancelStockWait while waiting: POST …/cancel {id, reason} with a key; CANCELLED → the next submit is a new exchange", async () => {
    const s = serverFor({ complete: [shortAnswer(), shortAnswer(), shortAnswer()] });
    s.on("POST", "/api/transactions/tx-1/cancel", () => txAnswer("tx-1", "CANCELLED", { cancelReason: "x" }));
    const { store } = await ready(s, "STAFF_DESK");
    await store.submit();
    await store.cancelStockWait();
    const cancel = s.callsTo("POST", "/api/transactions/tx-1/cancel");
    expect(cancel).toHaveLength(1);
    expect(cancel[0].key, "the cancel carries an Idempotency-Key").toBeTruthy();
    const body = JSON.parse(cancel[0].body);
    expect(body.id).toBe("tx-1");
    expect(typeof body.reason === "string" && body.reason.length > 0, "a reason is sent").toBe(true);
    expect(store.stockWait()).toBeUndefined();
    expect(store.transactionState()).toBe("CANCELLED");
    const n = s.callsTo("POST", "/api/transactions/tx-1/complete").length;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(s.callsTo("POST", "/api/transactions/tx-1/complete"), "no complete after the cancel").toHaveLength(n);
  });

  it("S6 stockInsufficient from create or confirm stays a refusal (create wrote nothing; only complete waits)", async () => {
    for (const flow of [{ create: [shortAnswer()] }, { confirm: [shortAnswer()] }] as Flow[]) {
      const s = serverFor(flow);
      const { store } = await ready(s, "STAFF_DESK");
      await expect(store.submit()).resolves.toBe("refused");
      expect(store.stockWait()).toBeUndefined();
      expect(s.callsTo("POST", "/api/transactions/tx-1/complete")).toHaveLength(0);
    }
  });

  it("S7 approvalRequired or staleVersion from complete is a refusal: no wait, no resend, no transaction read", async () => {
    for (const answer of [
      refusal(409, "approvalRequired", "approvalRequired: 167d8c93-7648-405c-b015-8fc53f176e4a"),
      refusal(409, "staleVersion", "staleVersion: a CANCELLED exchange is not completed"),
    ]) {
      const s = serverFor({ complete: [answer, txAnswer("tx-1", "COMPLETED")] });
      const { store } = await ready(s, "STAFF_DESK");
      await expect(store.submit()).resolves.toBe("refused");
      expect(store.stockWait()).toBeUndefined();
      await vi.advanceTimersByTimeAsync(60_000);
      expect(s.callsTo("POST", "/api/transactions/tx-1/complete"), "no resend").toHaveLength(1);
      expect(s.callsTo("GET", "/api/v3/views/transaction"), "no transaction read").toHaveLength(0);
    }
  });

  it("P1 the PENDING_APPROVAL wait reads the real view shape ({ transaction }) and goes on to complete once CONFIRMED (unchanged)", async () => {
    const s = serverFor({ confirm: [txAnswer("tx-1", "PENDING_APPROVAL", { approvalRequestId: "ar-1" })] });
    let reads = 0;
    s.on("GET", "/api/v3/views/transaction", () => (++reads === 1 ? txView("PENDING_APPROVAL") : txView("CONFIRMED")));
    const { store } = await ready(s, "STAFF_DESK");
    await store.submit();
    expect(s.callsTo("POST", "/api/transactions/tx-1/complete")).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(15_000);
    await vi.advanceTimersByTimeAsync(15_000);
    await vi.waitFor(() => expect(store.outcome()).toBe("saved"));
    expect(s.callsTo("POST", "/api/transactions/tx-1/complete")).toHaveLength(1);
  });
});
