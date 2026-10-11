// #543: steps 2–4 are wired into the real desk page. The page is booted exactly as `main.tsx` boots it (as
// `DeskPage.test.tsx` does for #539), against a fake server, and an exchange is walked from step 0 to step 4 through the
// real steps' own controls. Lesson from #539 (FAIL at ccb062a0): steps that pass their own tests but are not reached by
// the page are not done. Without `steps/booking`, `steps/payment` and `steps/receipt` the skip at step 1 lands on the
// page's empty panel and this test goes red at "step 2". Written before the implementation by the test-writer
// (claude/709ec749, on the FE lane controller claude/5edf7bb5's order). Do not edit, delete or weaken.
//
// [required]: the headings of steps 2–4 are v2's (§3.6b): `ข้อมูลการจอง`, `ชำระเงินและเอกสารบัญชี`,
// `ใบเสร็จรายการแลกเงิน`; their controls are the names pinned in `steps/{booking,payment}/step.test.tsx`.
// The answers are the generated types (`QuoteDto`, `TransactionDto`, `DeskConfig`). `…/complete` is TX-5's (#873, not
// merged): its fake answers a `TransactionDto` with `status: "COMPLETED"`, and only `id` and `status` are read.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setLang } from "../../../../shared/i18n";
import type { DeskConfig } from "../../../../shared/api/generated/desk_config";
import type { QuoteDto } from "../../../../shared/api/generated/quote";
import type { TransactionDto } from "../../../../shared/api/generated/transactions";

const HEAD = {
  rate: "เลือกเรทและจำนวนเงิน",
  customer: "พาสปอร์ต / ข้อมูลลูกค้า",
  booking: "ข้อมูลการจอง",
  payment: "ชำระเงินและเอกสารบัญชี",
  receipt: "ใบเสร็จรายการแลกเงิน",
};
const SKIP = "ข้ามข้อมูลลูกค้า →";
const DOC = "XC-20261011-000001";

const ME = { kind: "user", id: "u-1", role: "", branch: "MAIN", name: "Ploy", username: "ploy", branchName: "XC-B", branches: [] };

const CONFIG: DeskConfig = {
  branchId: "MAIN",
  branchName: "XC-B",
  shiftOpen: true,
  adminUrl: "/?branch=MAIN",
  rates: [{ code: "USD", unitSize: "1", buy: "35.12000000", sell: "35.42000000", publication: "pub-1" }],
  paymentMethods: [{ id: "Cash", enabled: true, buy: true, sell: true, account: "Cash drawer", instruction: "" }],
};

const QUOTE: QuoteDto = {
  id: "q-1",
  type: "BUY",
  channel: "DESK",
  expiresAt: "2099-01-01T00:00:00Z",
  publication: { id: "pub-1", version: 1 },
  lines: [
    {
      currency: "USD",
      shopSide: "BUY",
      quantity: "100",
      unitSize: "1",
      baseRate: "35.12000000",
      finalRate: "35.12000000",
      thbAmount: "3512.00",
      feeThb: "0.00",
      settleThb: "3512.00",
      publishedRateLineId: "line-1",
    },
  ],
  benefits: [],
  stale: false,
};

function tx(status: string, documentRef: string | null = null): TransactionDto {
  return {
    id: "tx-1",
    branchId: "MAIN",
    mode: "LIVE",
    status,
    documentRef,
    quoteId: "q-1",
    customerId: "c-1",
    approvalRequestId: null,
    cancelReason: null,
    cancelledAt: null,
    recordedAt: "2026-10-11T03:00:00Z",
    lines: [
      { currency: "USD", shopSide: "BUY", quantity: "100", unitSize: "1", rateSnapshot: "35.12000000", thbAmount: "3512.00", feeThb: "0.00", settleThb: "3512.00" },
    ],
    payment: { method: "Cash", reference: "—", account: "Cash drawer", amountThb: "3512.00", bankFeeThb: "0", paidAt: "2026-10-11T03:00:00Z" },
  };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

function fakeServer() {
  const calls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const method = String(init?.method ?? "GET").toUpperCase();
      const path = String(input).split("?")[0]!;
      calls.push(`${method} ${path}`);
      const body = typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
      if (path === "/api/v3/views/me") return json(200, { version: 1, changed: true, data: ME });
      if (path === "/api/v3/views/desk_config" || path === "/api/v3/views/staff_desk_config") {
        return json(200, { version: 1, changed: true, data: CONFIG });
      }
      if (method === "POST" && path === "/api/quotes") return json(200, QUOTE);
      if (method === "POST" && (path === "/api/rpc" || path === "/api/staff/rpc")) {
        if (body?.action === "reserveCustomer") return json(200, { customerId: "c-1", existing: false });
        if (body?.action === "opsBeginTransaction") return json(200, { staffDraftId: "d-1" });
      }
      if (method === "POST" && (path === "/api/upload" || path === "/api/staff/upload")) return json(200, { uploadKey: `k-${calls.length}` });
      if (method === "POST" && path === "/api/transactions") return json(200, tx("DRAFT"));
      if (method === "POST" && path === "/api/transactions/tx-1/confirm") return json(200, tx("CONFIRMED"));
      if (method === "POST" && path === "/api/transactions/tx-1/complete") return json(200, tx("COMPLETED", DOC));
      return json(404, { error: { code: "notFound", message: `unexpected ${method} ${path}` } });
    }),
  );
  return { calls };
}

let app: HTMLDivElement;

beforeEach(() => {
  setLang("th");
  vi.resetModules();
  vi.stubGlobal("confirm", vi.fn(() => true));
  app = document.createElement("div");
  app.id = "app";
  document.body.appendChild(app);
});

afterEach(() => {
  app.remove();
  window.history.replaceState({}, "", "/");
  vi.unstubAllGlobals();
});

const q = <T extends Element>(sel: string) => app.querySelector<T & HTMLElement>(sel);
const buttonText = (text: string) =>
  [...app.querySelectorAll<HTMLButtonElement>("button")].find((b) => (b.textContent ?? "").trim() === text);

function set(name: string, value: string) {
  const el = q<HTMLInputElement>(`[name="${name}"]`)!;
  expect(el, `control ${name}`).not.toBeNull();
  el.value = value;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  el.dispatchEvent(new Event("change", { bubbles: true }));
}

function submitForm() {
  const b = q<HTMLButtonElement>('#panel button[type="submit"]')!;
  expect(b, "the step's submit button").not.toBeNull();
  b.click();
}

async function waitHeading(text: string, label: string) {
  await vi.waitFor(() => expect(q("#panel")?.textContent ?? "", label).toContain(text), { timeout: 4000 });
}

describe("the desk page reaches steps 2–4 (#543)", () => {
  it("W1 the step bar lists all five steps, 01 … 05", async () => {
    fakeServer();
    window.history.replaceState({}, "", "/desk");
    await import("../../main");
    await waitHeading(HEAD.rate, "step 0");
    const items = [...app.querySelectorAll("ol.steps li")].map((li) => (li.textContent ?? "").slice(0, 2));
    expect(items).toEqual(["01", "02", "03", "04", "05"]);
  });

  it("W2 an exchange walks from step 0 to step 4 on the real page and ends with the server's document number", async () => {
    const server = fakeServer();
    window.history.replaceState({}, "", "/desk");
    await import("../../main");
    await waitHeading(HEAD.rate, "step 0");

    // step 0: currency and amount; the store asks for a quote
    await vi.waitFor(() => expect([...app.querySelectorAll('select[name="currency"] option')].map((o) => (o as HTMLOptionElement).value)).toContain("USD"));
    set("currency", "USD");
    set("amount", "100");
    await vi.waitFor(() => expect(server.calls).toContain("POST /api/quotes"), { timeout: 4000 });
    await vi.waitFor(() => expect(q("#panel")?.textContent ?? "").toContain("35.12000000"), { timeout: 4000 });
    submitForm();
    await waitHeading(HEAD.customer, "step 1");

    // step 1: skip the customer (v2's desk-skip.js), which reserves a walk-in and goes on
    buttonText(SKIP)!.click();
    await waitHeading(HEAD.booking, "step 2 (booking) is reached");

    // step 2 → 3
    submitForm();
    await waitHeading(HEAD.payment, "step 3 (payment) is reached");

    // step 3: Cash, the payment box, confirm (window.confirm answers yes to the count)
    const method = q<HTMLSelectElement>('select[name="method"]')!;
    method.value = "Cash";
    method.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() => expect(q<HTMLInputElement>('[name="account"]')?.value).toBe("Cash drawer"));
    const box = q<HTMLInputElement>('input[name="paymentChecked"]')!;
    if (!box.checked) box.click();
    submitForm();

    // step 4: the server's answer
    await waitHeading(HEAD.receipt, "step 4 (receipt) is reached");
    await vi.waitFor(() => expect(q("#panel")?.textContent ?? "").toContain(DOC), { timeout: 4000 });
    for (const call of ["POST /api/transactions", "POST /api/transactions/tx-1/confirm", "POST /api/transactions/tx-1/complete"]) {
      expect(server.calls, call).toContain(call);
    }
  }, 15000);
});
