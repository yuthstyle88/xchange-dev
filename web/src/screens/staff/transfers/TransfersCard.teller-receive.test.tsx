// #5957 (P0): the shift's teller sees the transfers into their own drawer on the staff page and receives them
// (design `docs/design/blueprint/cash-shift-transfer.md:93-95`: the receiving teller, `ShiftOperate`, own shift).
// The teller's list is `my_shift.incomingTransfers` (DISPATCHED transfers whose `toShiftId` is this shift); the
// teller cannot read `cash_transfers` (`cashTransfers` needs `CashManage`).
//
// Written before the implementation by the ticket author (claude/01TxaRJG). Do not edit, delete or weaken these
// tests; add your own in another file.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { setLang } from "../../../shared/i18n";
import {
  makeCashTransferRow,
  makeDenominationRow,
  makeMyShift,
  makeStaffFloorMyShift,
  makeStaffFloorView,
  makeStaffMeView,
  type StaffMeView,
} from "../../../shared/testing/fixtures";
import type { StaffCard, StaffCtx } from "../cards";
import { warmModules } from "../../../shared/testing/warm-modules";
import { th as transfersTh } from "./i18n";

const cardMods = import.meta.glob<{ card: StaffCard }>("./card.ts");

async function loadCard(): Promise<StaffCard> {
  const l = cardMods["./card.ts"];
  expect(l, "web/src/screens/staff/transfers/card.ts exists").toBeDefined();
  return (await l!()).card;
}

const BRANCH = "MAIN";
const SHIFT = "SH-0b765d57-0000-4000-8000-000000000001";
const OTHER_SHIFT = "SH-11111111-0000-4000-8000-000000000002";
const ME = "00000000-0000-4000-8000-0000000000c1";
const T_IN = "f0000000-0000-4000-8000-000000000001";

function me(permissions: string[]) {
  return makeStaffMeView({
    userId: ME,
    name: "Teller 1",
    roles: [],
    branchId: BRANCH,
    branchName: "XC-B",
    permissions,
  });
}

function floor(open: boolean) {
  return makeStaffFloorView({
    phase: "OPEN",
    hours: null,
    myClockIn: "2026-11-02T01:00:00Z",
    myShift: open ? makeStaffFloorMyShift({ id: SHIFT, openedAt: "2026-11-02T01:05:00Z" }) : null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

const DENOMS = [
  makeDenominationRow({ id: 164, currency: "THB", faceValue: "1000.0000", groupCode: "NOTE" }),
  makeDenominationRow({ id: 12, currency: "THB", faceValue: "100.0000", groupCode: "NOTE" }),
];

const INCOMING = makeCashTransferRow({
  id: T_IN,
  kind: "SAFE_TO_DRAWER",
  status: "DISPATCHED",
  fromBranchId: BRANCH,
  toBranchId: BRANCH,
  fromShiftId: null,
  toShiftId: SHIFT,
  createdBy: "00000000-0000-4000-8000-0000000000f1",
  createdAt: "2026-11-02T08:45:00Z",
  note: "Top up Teller 1 drawer",
  lines: [{ currency: "THB", denominationId: 164, qtySent: "5", qtyReceived: null }],
});

/** `my_shift` with the server field this ticket waits on (#5957 §2). */
function myShift(incoming: unknown[]) {
  return { ...makeMyShift({ id: SHIFT, status: "OPEN", openedAt: "2026-11-02T01:05:00Z", balances: [] }), incomingTransfers: incoming };
}

type Sent = { url: string; method: string; key: string | null; body: string | undefined };
type Answer = { status: number; body: unknown };

let sent: Sent[];
let reads: string[];
let answers: Record<string, Answer[]>;
/** Successive answers of `my_shift`; the last one repeats. */
let myShiftAnswers: unknown[];
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

function stub(views: Record<string, unknown> = {}) {
  const all: Record<string, unknown> = {
    cash_transfers: [],
    denominations: { rows: DENOMS },
    transfer_branches: { rows: [] },
    ...views,
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = url.split("?")[0]!;
      const method = (init?.method ?? "GET").toUpperCase();
      if (method === "GET") {
        const name = path.startsWith("/api/v3/views/") ? path.slice("/api/v3/views/".length) : "";
        reads.push(name || path);
        if (name === "my_shift") {
          const data = myShiftAnswers.length > 1 ? myShiftAnswers.shift() : myShiftAnswers[0];
          return json(200, { version: 1, changed: true, data });
        }
        if (name in all) return json(200, { version: 1, changed: true, data: all[name] });
        return json(404, { error: { code: "notFound", message: "not found" } });
      }
      const headers = new Headers(init?.headers);
      sent.push({ url: path, method, key: headers.get("idempotency-key"), body: init?.body as string | undefined });
      const next = answers[path]?.shift();
      if (next) return json(next.status, next.body);
      return json(200, { id: path.split("/")[3], status: "RECEIVED" });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(m: StaffMeView, open = true) {
  const card = await loadCard();
  const refresh = vi.fn(async () => {});
  const ctx: StaffCtx = { me: m, floor: floor(open), refresh };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
  return { refresh };
}

const tid = (id: string) => `[data-testid="${id}"]`;
const q = <T extends Element = HTMLElement>(sel: string) => host.querySelector<T>(sel);
const inRow = <T extends HTMLElement = HTMLElement>(id: string, control: string) =>
  q<T>(`[data-transfer-id="${id}"] ${tid(control)}`);

function typeInto(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

async function receiveFive() {
  inRow<HTMLButtonElement>(T_IN, "transfer-receive")!.click();
  await settle();
  const form = q<HTMLFormElement>(tid("transfer-receive-form"));
  expect(form, "receive form").not.toBeNull();
  typeInto(form!.querySelector<HTMLInputElement>('input[data-denomination-id="164"]')!, "5");
  q<HTMLButtonElement>(tid("transfer-receive-save"))!.click();
  await settle();
}

const SHIFT_ONLY = ["ShiftOperate", "ExchangeTransact"];

beforeAll(() => warmModules(() => import("./card"), () => import("../main")), 60_000);

beforeEach(() => {
  sent = [];
  reads = [];
  answers = {};
  myShiftAnswers = [myShift([INCOMING])];
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  setLang("th");
});

describe("#5957 card visibility", () => {
  it("V1: ShiftOperate with an open shift sees the card", async () => {
    const card = await loadCard();
    expect(card.visible({ me: me(SHIFT_ONLY), floor: floor(true), refresh: async () => {} })).toBe(true);
  });

  it("V2: ShiftOperate with no open shift does not", async () => {
    const card = await loadCard();
    expect(card.visible({ me: me(SHIFT_ONLY), floor: floor(false), refresh: async () => {} })).toBe(false);
  });

  it("V3: CashManage sees it, as today", async () => {
    const card = await loadCard();
    expect(card.visible({ me: me(["CashManage"]), floor: floor(false), refresh: async () => {} })).toBe(true);
  });

  it("V4: neither permission: not visible", async () => {
    const card = await loadCard();
    expect(card.visible({ me: me(["ExchangeTransact"]), floor: floor(true), refresh: async () => {} })).toBe(false);
  });
});

describe("#5957 the teller's mode", () => {
  it("T1: lists my_shift.incomingTransfers with only a receive action, and never reads cash_transfers", async () => {
    stub();
    await mount(me(SHIFT_ONLY));
    await vi.waitFor(() => expect(inRow(T_IN, "transfer-receive")).not.toBeNull());

    expect(reads).toContain("my_shift");
    expect(reads).not.toContain("cash_transfers");
    expect(q(tid("transfer-new")), "no create button").toBeNull();
    expect(q(tid("transfer-form")), "no create form").toBeNull();
    expect(q(tid("transfer-dispatch")), "no dispatch").toBeNull();
    expect(q(tid("transfer-reconcile")), "no reconcile").toBeNull();
  });

  it("T2: receive sends one keyed POST with the counted lines, then re-reads my_shift and the row is gone", async () => {
    myShiftAnswers = [myShift([INCOMING]), myShift([])];
    stub();
    const { refresh } = await mount(me(SHIFT_ONLY));
    await vi.waitFor(() => expect(inRow(T_IN, "transfer-receive")).not.toBeNull());
    const before = reads.filter((r) => r === "my_shift").length;

    await receiveFive();

    expect(sent).toHaveLength(1);
    expect(sent[0]!.url).toBe(`/api/cash-transfers/${T_IN}/receive`);
    expect(sent[0]!.method).toBe("POST");
    expect(sent[0]!.key, "Idempotency-Key").toBeTruthy();
    expect(JSON.parse(sent[0]!.body!)).toEqual({
      branch: BRANCH,
      id: T_IN,
      lines: [{ currency: "THB", denominationId: 164, qtyReceived: "5" }],
    });
    await vi.waitFor(() => expect(reads.filter((r) => r === "my_shift").length).toBeGreaterThan(before));
    await vi.waitFor(() => expect(q(`[data-transfer-id="${T_IN}"]`)).toBeNull());
    expect(q(tid("transfer-notice"))?.textContent).toBe(transfersTh.saved);
    expect(refresh).toHaveBeenCalled();
  });

  it("T3: a refused receive is shown worded; the form and the row stay", async () => {
    stub();
    answers[`/api/cash-transfers/${T_IN}/receive`] = [
      {
        status: 400,
        body: { error: { code: "validationFailed", message: "only the shift's teller may receive into drawer" } },
      },
    ];
    await mount(me(SHIFT_ONLY));
    await vi.waitFor(() => expect(inRow(T_IN, "transfer-receive")).not.toBeNull());

    await receiveFive();

    expect(sent).toHaveLength(1);
    const err = q(tid("transfer-error"));
    expect(err, "transfer-error shown").not.toBeNull();
    expect(err!.textContent ?? "").not.toBe("");
    expect(q(tid("transfer-receive-form")), "form stays open").not.toBeNull();
    expect(q(`[data-transfer-id="${T_IN}"]`), "row stays").not.toBeNull();
  });

  it("T4: no incoming transfer: the empty line and no rows", async () => {
    myShiftAnswers = [myShift([])];
    stub();
    await mount(me(SHIFT_ONLY));
    await settle();
    expect(reads).toContain("my_shift");
    expect(reads).not.toContain("cash_transfers");
    expect(host.querySelectorAll("[data-transfer-id]")).toHaveLength(0);
    expect(host.textContent ?? "").toContain(transfersTh.empty);
  });

  it("edge: my_shift answers null (the shift closed meanwhile): the empty line, no error", async () => {
    myShiftAnswers = [null];
    stub();
    await mount(me(SHIFT_ONLY));
    await settle();
    expect(reads).toContain("my_shift");
    expect(host.querySelectorAll("[data-transfer-id]")).toHaveLength(0);
    expect(q(tid("transfer-error"))).toBeNull();
  });

  it("edge: a transfer for another shift is not listed even if the read carried it", async () => {
    myShiftAnswers = [myShift([{ ...INCOMING, toShiftId: OTHER_SHIFT }])];
    stub();
    await mount(me(SHIFT_ONLY));
    await settle();
    expect(reads).toContain("my_shift");
    expect(q(`[data-transfer-id="${T_IN}"]`)).toBeNull();
  });
});

describe("#5957 CashManage holders keep today's card", () => {
  it("K1: CashManage (with or without ShiftOperate) reads cash_transfers, not my_shift", async () => {
    stub({ cash_transfers: [INCOMING] });
    await mount(me(["CashManage", "ShiftOperate"]));
    await vi.waitFor(() => expect(reads).toContain("cash_transfers"));
    expect(reads).not.toContain("my_shift");
  });
});
