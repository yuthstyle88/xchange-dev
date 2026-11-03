// F5 FE (#564 finding F5; server side PR 6043 / #6037, design docs/design/blueprint/cash-shift-transfer.md §4.10):
// a submitted transfer says it waits for approval, and a rejected one says it was not approved.
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests (the staff transfers card; the status cell, the third cell of `[data-transfer-id]`):
// - P1: `status: DRAFT` + `approvalStatus: PENDING` ⇒ "รออนุมัติ" (owner-approved, #564 issuecomment-6596705394).
// - P2: `DRAFT` + `REJECTED` ⇒ "ไม่อนุมัติ" (owner-approved).
// - P3: `DRAFT` + `null` (never submitted) ⇒ "ร่าง", as today.
// - P4: any other status reads as today, whatever `approvalStatus` says (`APPROVED` + `APPROVED` ⇒ "อนุมัติแล้ว").
// - P5: in English, "Awaiting approval" and "Not approved".
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { setLang } from "../../../shared/i18n";
import {
  makeCashTransferRow,
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffMeView,
} from "../../../shared/testing/fixtures";
import type { StaffCtx } from "../cards";
import { warmModules } from "../../../shared/testing/warm-modules";
import { card } from "./card";

const BRANCH = "MAIN";

const ME = makeStaffMeView({
  userId: "00000000-0000-4000-8000-0000000000b1",
  name: "Ploy",
  roles: [],
  branchId: BRANCH,
  branchName: "XC-B",
  permissions: ["CashManage", "FinanceReconcile"],
});

const FLOOR = makeStaffFloorView({
  phase: "OPEN",
  hours: null,
  myClockIn: "2026-10-11T01:00:00Z",
  myShift: null,
  cutoff: null,
  target: null,
  safe: null,
});

const DENOMS = [makeDenominationRow({ id: 21, currency: "USD", faceValue: "50.0000", groupCode: "NOTE" })];

function transfer(id: string, status: string, approvalStatus: string | null) {
  return makeCashTransferRow({
    id,
    kind: "SAFE_TO_DRAWER",
    fromBranchId: BRANCH,
    toBranchId: null,
    fromShiftId: null,
    toShiftId: "S-1",
    status,
    approvalRequestId: approvalStatus ? `ap-${id}` : null,
    approvalStatus,
    createdBy: "00000000-0000-4000-8000-0000000000b2",
    note: "n",
    createdAt: "2026-11-03T02:00:00Z",
    lines: [],
  });
}

const T_PENDING = "e0000000-0000-4000-8000-000000000001";
const T_REJECTED = "e0000000-0000-4000-8000-000000000002";
const T_DRAFT = "e0000000-0000-4000-8000-000000000003";
const T_APPROVED = "e0000000-0000-4000-8000-000000000004";

const TRANSFERS = [
  transfer(T_PENDING, "DRAFT", "PENDING"),
  transfer(T_REJECTED, "DRAFT", "REJECTED"),
  transfer(T_DRAFT, "DRAFT", null),
  transfer(T_APPROVED, "APPROVED", "APPROVED"),
];

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

function stub() {
  const views: Record<string, unknown> = {
    cash_transfers: TRANSFERS,
    denominations: { rows: DENOMS },
    transfer_branches: { rows: [] },
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = url.split("?")[0]!;
      const name = path.startsWith("/api/v3/views/") ? path.slice("/api/v3/views/".length) : "";
      if (name in views) return json(200, { version: 1, changed: true, data: views[name] });
      return json(404, { error: { code: "notFound", message: "not found" } });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 4; i++) {
    for (let j = 0; j < 5; j++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

async function mount() {
  const ctx: StaffCtx = { me: ME, floor: FLOOR, refresh: vi.fn(async () => {}) };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
  await vi.waitFor(() => expect(host.querySelector(`[data-transfer-id="${T_PENDING}"]`)).not.toBeNull());
}

const statusOf = (id: string): string => {
  const row = host.querySelector(`[data-transfer-id="${id}"]`);
  expect(row, `the row of ${id}`).not.toBeNull();
  return (row!.querySelectorAll("td")[2]?.textContent ?? "").trim();
};

beforeAll(() => warmModules(() => import("./card")), 60_000);

beforeEach(() => {
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
  stub();
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  setLang("th");
});

describe("F5 the staff transfers card names the approval state of a draft", () => {
  it("P1 a draft waiting for approval reads รออนุมัติ", async () => {
    await mount();
    expect(statusOf(T_PENDING)).toBe("รออนุมัติ");
  });

  it("P2 a draft whose approval was rejected reads ไม่อนุมัติ", async () => {
    await mount();
    expect(statusOf(T_REJECTED)).toBe("ไม่อนุมัติ");
  });

  it("P3 a draft never submitted still reads ร่าง", async () => {
    await mount();
    expect(statusOf(T_DRAFT)).toBe("ร่าง");
  });

  it("P4 an approved transfer reads as today", async () => {
    await mount();
    expect(statusOf(T_APPROVED)).toBe("อนุมัติแล้ว");
  });

  it("P5 in English: Awaiting approval, Not approved, Draft", async () => {
    setLang("en");
    await mount();
    expect(statusOf(T_PENDING)).toBe("Awaiting approval");
    expect(statusOf(T_REJECTED)).toBe("Not approved");
    expect(statusOf(T_DRAFT)).toBe("Draft");
  });
});
