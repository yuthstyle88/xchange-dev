// Receive gate on the staff transfers card, CashManage path (#564 issuecomment-6604004214): the server lets only the
// teller of a transfer's open `toShiftId` shift receive it (ruling Q1,
// api/crates/xc-cash/src/application/transfer/transfer_flow.rs:736-750). The card must not draw what the server refuses.
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or weaken these tests;
// add your own elsewhere.
//
// [required] by these tests (a CashManage viewer; `[data-transfer-id] [data-testid="transfer-receive"]`, DISPATCHED rows):
// - M1: a row with `toShiftId` equal to the viewer's own shift (`ctx.floor.myShift.id`) draws receive.
// - M2: a row with another `toShiftId` does not.
// - M3: the viewer has no open shift: no row with `toShiftId` draws receive.
// - M4: a row without `toShiftId` (BRANCH into this branch) keeps today's receive.
// The teller path (no CashManage, `my_shift.incomingTransfers`) is unchanged and pinned by TransfersCard.teller-receive.test.tsx.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { setLang } from "../../../shared/i18n";
import {
  makeCashTransferRow,
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffFloorMyShift,
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

const floor = (myShiftId: string | null) => makeStaffFloorView({
  phase: "OPEN",
  hours: null,
  myClockIn: "2026-10-11T01:00:00Z",
  myShift: myShiftId ? makeStaffFloorMyShift({ id: myShiftId, openedAt: "2026-10-11T01:00:00Z" }) : null,
  cutoff: null,
  target: null,
  safe: null,
});

const DENOMS = [makeDenominationRow({ id: 21, currency: "USD", faceValue: "50.0000", groupCode: "NOTE" })];

function transfer(id: string, kind: string, toShiftId: string | null, toBranchId: string | null, fromBranchId = BRANCH) {
  return makeCashTransferRow({
    id,
    kind,
    fromBranchId,
    toBranchId,
    fromShiftId: null,
    toShiftId,
    status: "DISPATCHED",
    approvalRequestId: `ap-${id}`,
    approvalStatus: "APPROVED",
    createdBy: "00000000-0000-4000-8000-0000000000b2",
    note: "n",
    createdAt: "2026-11-03T02:00:00Z",
    lines: [],
  });
}

const T_MINE = "e0000000-0000-4000-8000-000000000011";
const T_OTHER = "e0000000-0000-4000-8000-000000000012";
const T_BRANCH = "e0000000-0000-4000-8000-000000000013";

const TRANSFERS = [
  transfer(T_MINE, "SAFE_TO_DRAWER", "S-1", null),
  transfer(T_OTHER, "SAFE_TO_DRAWER", "S-2", null),
  transfer(T_BRANCH, "BRANCH", null, BRANCH, "OTHER"),
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

async function mount(myShiftId: string | null) {
  const ctx: StaffCtx = { me: ME, floor: floor(myShiftId), refresh: vi.fn(async () => {}) };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
  await vi.waitFor(() => expect(host.querySelector(`[data-transfer-id="${T_BRANCH}"]`)).not.toBeNull());
}

const receiveOf = (id: string) => host.querySelector(`[data-transfer-id="${id}"] [data-testid="transfer-receive"]`);

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

describe("the staff transfers card draws receive only for the teller of the transfer's shift", () => {
  it("M1 a transfer into my own shift: receive", async () => {
    await mount("S-1");
    expect(receiveOf(T_MINE)).not.toBeNull();
  });

  it("M2 a transfer into another shift: no receive", async () => {
    await mount("S-1");
    expect(receiveOf(T_OTHER)).toBeNull();
  });

  it("M3 no open shift of mine: no receive on any drawer transfer", async () => {
    await mount(null);
    expect(receiveOf(T_MINE)).toBeNull();
    expect(receiveOf(T_OTHER)).toBeNull();
  });

  it("M4 a branch transfer without toShiftId keeps receive", async () => {
    await mount(null);
    expect(receiveOf(T_BRANCH)).not.toBeNull();
  });
});
