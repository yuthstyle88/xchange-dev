// #3321: ShiftCard handleVerifyCount handles AWAITING_FINANCE response
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { errorText, setLang } from "../../../shared/i18n";
import { th, en } from "./i18n";
import type { StaffFloorView } from "../../../shared/api/generated/staff_floor";
import type { StaffMeView } from "../../../shared/api/generated/staff_me";
import type { CashCountRow } from "../../../shared/api/generated/cash_counts";
import type { DenominationRow } from "../../../shared/api/generated/denominations";
import type { HandoverPendingRow } from "../../../shared/api/generated/handovers_pending";
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

function me(permissions: string[]): StaffMeView {
  return {
    userId: ME,
    name: "Ploy",
    roles: [],
    branchId: BRANCH,
    branchName: "XC-B",
    permissions,
  };
}

function floor(over: Partial<StaffFloorView> = {}): StaffFloorView {
  return {
    phase: "OPEN",
    hours: null,
    myClockIn: null,
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
    ...over,
  };
}

const C_OPEN = "c0000000-0000-4000-8000-000000000001";

const COUNTS: CashCountRow[] = [
  {
    id: C_OPEN,
    branchId: BRANCH,
    shiftId: "22222222-2222-4222-8222-222222222222",
    kind: "OPEN",
    countedBy: OTHER,
    countedAt: "2026-10-11T02:00:00Z",
    verifiedBy: null,
    verifiedAt: null,
    note: null,
    totals: [],
  },
];

const DENOMS: DenominationRow[] = [
  { id: 11, currency: "THB", faceValue: "1000.0000", groupCode: "NOTE" },
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
    handovers_pending: [] as HandoverPendingRow[],
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

describe("#3321 ShiftCard count verify: AWAITING_FINANCE vs VERIFIED", () => {
  it("row 1: verify answers {status: 'VERIFIED'} => notice Saved (kept in TH and EN)", async () => {
    stub();
    answers[`/api/cash-counts/${C_OPEN}/verify`] = [
      { status: 200, body: { id: C_OPEN, status: "VERIFIED", approvalRequestId: null } },
    ];
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());

    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() =>
      expect(must(tid("shift-notice")).textContent?.trim()).toBe(th.saved),
    );

    // EN check
    unmount();
    setLang("en");
    answers[`/api/cash-counts/${C_OPEN}/verify`] = [
      { status: 200, body: { id: C_OPEN, status: "VERIFIED", approvalRequestId: null } },
    ];
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());

    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() =>
      expect(must(tid("shift-notice")).textContent?.trim()).toBe(en.saved),
    );
  });

  it("row 2: verify answers {status: 'AWAITING_FINANCE', approvalRequestId: 'ar-1'} => notice holds awaiting text and ar-1, never Saved", async () => {
    stub();
    answers[`/api/cash-counts/${C_OPEN}/verify`] = [
      { status: 200, body: { id: C_OPEN, status: "AWAITING_FINANCE", approvalRequestId: "ar-1" } },
    ];
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());

    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() => expect(q(tid("shift-notice"))).not.toBeNull());

    const noticeTh = must(tid("shift-notice")).textContent?.trim();
    const expectedTh = `${th.countAwaitingFinance} (${th.approvalRequest} ar-1)`;
    expect(noticeTh).toBe(expectedTh);
    expect(noticeTh).not.toContain(th.saved);

    // EN check
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
    const expectedEn = `${en.countAwaitingFinance} (${en.approvalRequest} ar-1)`;
    expect(noticeEn).toBe(expectedEn);
    expect(noticeEn).not.toContain(en.saved);
  });

  it("row 2 / §9 edge case: approvalRequestId null with AWAITING_FINANCE => awaiting text with empty id", async () => {
    stub();
    answers[`/api/cash-counts/${C_OPEN}/verify`] = [
      { status: 200, body: { id: C_OPEN, status: "AWAITING_FINANCE", approvalRequestId: null } },
    ];
    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());

    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() => expect(q(tid("shift-notice"))).not.toBeNull());

    const notice = must(tid("shift-notice")).textContent?.trim();
    const expected = `${th.countAwaitingFinance} (${th.approvalRequest} )`;
    expect(notice).toBe(expected);
    expect(notice).not.toContain(th.saved);
  });

  it("row 3: verify answers 4xx => worded error, notice is cleared", async () => {
    stub();
    answers[`/api/cash-counts/${C_OPEN}/verify`] = [
      {
        status: 400,
        body: { error: { code: "roleNotAllowedHere", message: "refused by role" } },
      },
    ];
    const { refresh } = await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());

    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() => expect(q(tid("shift-error"))).not.toBeNull());

    expect(q(tid("shift-notice"))).toBeNull();
    expect(must(tid("shift-error")).textContent?.trim()).toBe(
      errorText("roleNotAllowedHere", "refused by role"),
    );
    expect(refresh).not.toHaveBeenCalled();
  });

  it("row 4: after either 200 answer, counts and the floor are re-read", async () => {
    // 4a. VERIFIED case
    stub();
    answers[`/api/cash-counts/${C_OPEN}/verify`] = [
      { status: 200, body: { id: C_OPEN, status: "VERIFIED", approvalRequestId: null } },
    ];
    const mountA = await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());

    const beforeA = reads.filter((r) => r === "cash_counts").length;
    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() => expect(q(tid("shift-notice"))).not.toBeNull());

    expect(mountA.refresh).toHaveBeenCalledTimes(1);
    expect(reads.filter((r) => r === "cash_counts").length).toBeGreaterThan(beforeA);

    // 4b. AWAITING_FINANCE case
    unmount();
    answers[`/api/cash-counts/${C_OPEN}/verify`] = [
      { status: 200, body: { id: C_OPEN, status: "AWAITING_FINANCE", approvalRequestId: "ar-99" } },
    ];
    const mountB = await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_OPEN)).not.toBeNull());

    const beforeB = reads.filter((r) => r === "cash_counts").length;
    await click(verifyOf(C_OPEN)!);
    await vi.waitFor(() => expect(q(tid("shift-notice"))).not.toBeNull());

    expect(mountB.refresh).toHaveBeenCalledTimes(1);
    expect(reads.filter((r) => r === "cash_counts").length).toBeGreaterThan(beforeB);
  });
});
