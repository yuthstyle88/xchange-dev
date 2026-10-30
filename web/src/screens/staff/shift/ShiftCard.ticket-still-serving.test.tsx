// #5705: ShiftCard shows ticketStillServing on refused close verify or hand-over accept
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { errorText, setLang } from "../../../shared/i18n";
import {
  makeCashCountRow,
  makeDenominationRow,
  makeHandoverPendingRow,
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

const C_CLOSE = "c0000000-0000-4000-8000-000000000002";

const CLOSE_COUNT = makeCashCountRow({
  id: C_CLOSE,
  branchId: BRANCH,
  shiftId: "22222222-2222-4222-8222-222222222222",
  kind: "CLOSE",
  status: "WAITING",
  countedBy: OTHER,
  countedAt: "2026-10-11T02:00:00Z",
  verifiedBy: null,
  verifiedAt: null,
  note: null,
  totals: [],
});

const H_MINE = "d0000000-0000-4000-8000-000000000001";

const HANDOVERS = [
  makeHandoverPendingRow({
    id: H_MINE,
    fromShiftId: "33333333-3333-4333-8333-333333333333",
    countId: "44444444-4444-4444-8444-444444444444",
    receiverId: ME,
    status: "PENDING",
    createdAt: "2026-10-11T03:00:00Z",
  }),
];

const DENOMS = [
  makeDenominationRow({ id: 11, currency: "THB", faceValue: "1000.0000", groupCode: "NOTE" }),
];

const SERVING_REFUSAL_MESSAGE =
  "ปิดกะไม่ได้ ยังมีคิวที่เรียกหรือกำลังให้บริการค้างอยู่ กรุณาจบคิวนั้นก่อน (เสร็จสิ้น ไม่มีการขาย หรือข้ามคิว)";

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
    cash_counts: [],
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
      return json(200, { id: path.split("/")[3] ?? "ok", status: "OK" });
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

function acceptOf(id: string): HTMLElement | null {
  return q(`[data-handover-id="${id}"] ${tid("shift-accept")}`);
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

describe("#5705 ShiftCard 409 ticketStillServing refusal", () => {
  it("a manager verifies a waiting CLOSE count, and the mock answers 409 ticketStillServing", async () => {
    stub({ cash_counts: [CLOSE_COUNT] });
    answers[`/api/cash-counts/${C_CLOSE}/verify`] = [
      {
        status: 409,
        body: {
          error: {
            code: "ticketStillServing",
            message: SERVING_REFUSAL_MESSAGE,
          },
        },
      },
    ];

    await mount(me(["ShiftCloseCertify"]), floor());
    await vi.waitFor(() => expect(verifyOf(C_CLOSE)).not.toBeNull());

    await click(verifyOf(C_CLOSE)!);
    await vi.waitFor(() => expect(q(tid("shift-error"))).not.toBeNull());

    expect(must(tid("shift-error")).textContent?.trim()).toBe(
      errorText("ticketStillServing", ""),
    );
    expect(q(`[data-count-id="${C_CLOSE}"]`)).not.toBeNull();
    expect(verifyOf(C_CLOSE)).not.toBeNull();
    expect(q(tid("shift-notice"))).toBeNull();
  });

  it("a receiver accepts a PENDING hand-over, with the same 409 ticketStillServing", async () => {
    stub({ handovers_pending: HANDOVERS });
    answers[`/api/handovers/${H_MINE}/accept`] = [
      {
        status: 409,
        body: {
          error: {
            code: "ticketStillServing",
            message: SERVING_REFUSAL_MESSAGE,
          },
        },
      },
    ];

    await mount(me(["ShiftOperate"]), floor());
    await vi.waitFor(() => expect(acceptOf(H_MINE)).not.toBeNull());

    await click(acceptOf(H_MINE)!);
    await vi.waitFor(() => expect(q(tid("shift-error"))).not.toBeNull());

    expect(must(tid("shift-error")).textContent?.trim()).toBe(
      errorText("ticketStillServing", ""),
    );
    expect(q(`[data-handover-id="${H_MINE}"]`)).not.toBeNull();
    expect(acceptOf(H_MINE)).not.toBeNull();
    expect(q(tid("shift-notice"))).toBeNull();
  });
});
