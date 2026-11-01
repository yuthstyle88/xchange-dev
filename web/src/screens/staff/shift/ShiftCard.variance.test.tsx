// FE-T4 #5850 (docs/design/fe-triage-1101.md §3 W9): the manager's waiting list and the AWAITING_FINANCE notice
// show the count difference per currency, never the approval request id.
// Harness copied from ShiftCard.awaiting-finance.test.tsx. Written before the implementation by the ticket author
// claude/01TxaRJG. Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { setLang } from "../../../shared/i18n";
import { th, en } from "./i18n";
import {
  makeCashCountRow,
  makeCashCountTotalRow,
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffMeView,
  type StaffFloorView,
  type StaffMeView,
} from "../../../shared/testing/fixtures";
import type { StaffCard, StaffCtx } from "../cards";
import { warmModules } from "../../../shared/testing/warm-modules";

const cardMods = import.meta.glob<{ card: StaffCard }>("./card.ts");

async function loadCard(): Promise<StaffCard> {
  const l = cardMods["./card.ts"];
  expect(l, "web/src/screens/staff/shift/card.ts exists").toBeDefined();
  return (await l!()).card;
}

const BRANCH = "MAIN";
const ME = "00000000-0000-4000-8000-0000000000a1";
const OTHER = "00000000-0000-4000-8000-0000000000a2";

function me(permissions: string[]) {
  return makeStaffMeView({
    userId: ME,
    name: "Ploy",
    roles: [],
    branchId: BRANCH,
    branchName: "XC-B",
    permissions,
  });
}

function floor(over: Partial<StaffFloorView> = {}) {
  return makeStaffFloorView({
    phase: "OPEN",
    hours: null,
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
    ...over,
  });
}

const C_OPEN = "c0000000-0000-4000-8000-000000000001";

const COUNTS = [
  makeCashCountRow({
    id: C_OPEN,
    branchId: BRANCH,
    shiftId: "22222222-2222-4222-8222-222222222222",
    kind: "OPEN",
    countedBy: OTHER,
    countedAt: "2026-10-11T02:00:00Z",
    verifiedBy: null,
    verifiedAt: null,
    note: null,
    totals: [
      makeCashCountTotalRow({ currency: "THB", counted: "4000", expected: "5000", difference: "-1000" }),
      makeCashCountTotalRow({ currency: "USD", counted: "10", expected: "10", difference: "0" }),
    ],
  }),
];

const C_EVEN = "c0000000-0000-4000-8000-000000000002";
const EVEN = makeCashCountRow({
  id: C_EVEN,
  branchId: BRANCH,
  shiftId: "33333333-3333-4333-8333-333333333333",
  kind: "CLOSE",
  countedBy: OTHER,
  countedAt: "2026-10-11T03:00:00Z",
  totals: [makeCashCountTotalRow({ currency: "THB", counted: "5000", expected: "5000", difference: "0" })],
});

const DENOMS = [
  makeDenominationRow({ id: 11, currency: "THB", faceValue: "1000.0000", groupCode: "NOTE" }),
];

type Sent = {
  url: string;
  method: string;
  key: string | null;
  body: string | undefined;
};
type Answer = { status: number; body: unknown } | "network";

let sent: Sent[];
let reads: string[];
let answers: Record<string, Answer[]>;
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function stub(views: Record<string, unknown> = {}) {
  const all: Record<string, unknown> = {
    cash_counts: COUNTS,
    handovers_pending: [],
    my_shift: null,
    denominations: { rows: DENOMS },
    handover_receivers: { rows: [] },
    ...views,
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const path = url.split("?")[0]!;
      const method = (init?.method ?? "GET").toUpperCase();
      if (method === "GET") {
        const name = path.startsWith("/api/v3/views/")
          ? path.slice("/api/v3/views/".length)
          : "";
        reads.push(name || path);
        if (name in all)
          return json(200, { version: 1, changed: true, data: all[name] });
        if (path === "/health/ready") {
          return json(200, {
            version: "3.0.0",
            build: "abc1234",
            channel: "staging",
            builtAt: "2026-10-11T00:00:00Z",
          });
        }
        return json(404, { error: { code: "notFound", message: "not found" } });
      }
      const headers = new Headers(init?.headers);
      sent.push({
        url: path,
        method,
        key: headers.get("idempotency-key"),
        body: init?.body as string | undefined,
      });
      const next = answers[path]?.shift();
      if (next === "network") throw new TypeError("Failed to fetch");
      if (next) return json(next.status, next.body);
      return json(200, { id: path.split("/")[3] ?? "ok", status: "VERIFIED", approvalRequestId: null });
    }),
  );
}

function ctxOf(m: StaffMeView, f: StaffFloorView | undefined) {
  const refresh = vi.fn(async () => {});
  const ctx: StaffCtx = { me: m, floor: f, refresh };
  return { ctx, refresh };
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(m: StaffMeView, f: StaffFloorView | undefined) {
  const card = await loadCard();
  const { ctx, refresh } = ctxOf(m, f);
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
  return { refresh };
}

function unmount() {
  dispose?.();
  dispose = undefined;
  host.innerHTML = "";
}

function q<T extends Element = HTMLElement>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function must<T extends Element = HTMLElement>(sel: string): T {
  const el = q<T>(sel);
  expect(el, sel).not.toBeNull();
  return el!;
}

function tid(id: string): string {
  return `[data-testid="${id}"]`;
}

function verifyOf(id: string): HTMLElement | null {
  return q(`[data-count-id="${id}"] ${tid("shift-verify")}`);
}

async function click(el: HTMLElement) {
  el.click();
  await settle();
}

beforeAll(
  () =>
    warmModules(
      () => import("./card"),
      () => import("../main"),
    ),
  60_000,
);

beforeEach(() => {
  sent = [];
  reads = [];
  answers = {};
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
});

afterEach(() => {
  unmount();
  host.remove();
  vi.unstubAllGlobals();
});

describe("FE-T4 #5850 W9 manager list and verify notice", () => {
  it("L1 the waiting list row shows the non-zero difference per currency in its sub-line", async () => {
    stub({ cash_counts: [...COUNTS, EVEN] });
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());

    const sub = must(`[data-count-id="${C_OPEN}"] .xc-shift-item-sub`);
    const diffs = [...sub.querySelectorAll("[data-count-difference]")].map((el) => [
      el.getAttribute("data-count-difference"),
      el.textContent?.trim(),
    ]);
    expect(diffs).toEqual([["THB", "ผลต่าง THB -1,000"]]);
    expect(sub.textContent).toContain("THB 4000");
    expect(sub.textContent).not.toContain(OTHER);

    const even = must(`[data-count-id="${C_EVEN}"] .xc-shift-item-sub`);
    expect(even.querySelectorAll("[data-count-difference]")).toHaveLength(0);
  });

  it("F1 AWAITING_FINANCE: the notice shows the count's difference, not the approval request id (th and en)", async () => {
    stub();
    answers[`/api/cash-counts/${C_OPEN}/verify`] = [
      { status: 200, body: { id: C_OPEN, status: "AWAITING_FINANCE", approvalRequestId: "ar-1" } },
    ];
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());
    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() => expect(q(tid("shift-notice"))).not.toBeNull());

    const noticeTh = must(tid("shift-notice")).textContent?.trim();
    expect(noticeTh).toBe(`${th.countAwaitingFinance} (ผลต่าง THB -1,000)`);
    expect(noticeTh).not.toContain("ar-1");
    expect(noticeTh).not.toContain("คำขออนุมัติ");

    unmount();
    setLang("en");
    answers[`/api/cash-counts/${C_OPEN}/verify`] = [
      { status: 200, body: { id: C_OPEN, status: "AWAITING_FINANCE", approvalRequestId: "ar-1" } },
    ];
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());
    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() => expect(q(tid("shift-notice"))).not.toBeNull());

    const noticeEn = must(tid("shift-notice")).textContent?.trim();
    expect(noticeEn).toBe(`${en.countAwaitingFinance} (Difference THB -1,000)`);
    expect(noticeEn).not.toContain("ar-1");
  });

  it("F2 AWAITING_FINANCE for a count whose differences are all zero: the awaiting text alone", async () => {
    stub({ cash_counts: [EVEN] });
    answers[`/api/cash-counts/${C_EVEN}/verify`] = [
      { status: 200, body: { id: C_EVEN, status: "AWAITING_FINANCE", approvalRequestId: "ar-2" } },
    ];
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_EVEN)).not.toBeNull());
    await click(verifyOf(C_EVEN)!);
    await vi.waitFor(() => expect(q(tid("shift-notice"))).not.toBeNull());

    expect(must(tid("shift-notice")).textContent?.trim()).toBe(th.countAwaitingFinance);
  });
});
