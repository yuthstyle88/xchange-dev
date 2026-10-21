// S-6 #579: the staff transfers card (the branch's transfers; create, submit, dispatch, receive, reconcile).
// Written before the implementation by the up-front test writer (claude/709ec749, on the xchange FE lane controller
// claude/5edf7bb5's order, the owner having ordered it on 2026-10-11). Do not edit, delete or weaken; add tests in a
// file of your own.
//
// Design: docs/design/xchange-v3-wave3-staff.md (SD)
//   §4      transfers/: transfers of the branch (cash_transfers): create, submit, dispatch, receive (with received
//           quantities), reconcile; drawer↔safe and external moves are the same flow (#582 C5). Texts staff.transfers.*.
//   §6.1    B8 list can(CashManage); create / dispatch / receive can(CashManage); reconcile can(FinanceReconcile).
//           (§11 O5, a teller receiving SAFE_TO_DRAWER without CashManage, is open and not pinned here.)
//   §6.2    C8 transferCreate then transferSubmit; C9 transferDispatch {id} / transferReceive {id, lines:
//           [{…, qtyReceived: str}]} / transferReconcile {id, note} (ReasonDialog). After success: notice "บันทึกแล้ว",
//           staff_floor and the card's lists re-read; on refusal t('error.'+code), nothing re-read, the form keeps its
//           values. Approvals are decided on PR 570's approvals screen, not here (SD §3.3).
//   S2 no role literal · S5 nothing through /api/staff/rpc · S8 TH/EN · §8.1 no Number-constructor call in staff/**.
// CA-8 #624 (cash-shift-transfer.md §4.4, §4.7; its provided tests @ 630b95f3 and its rulings):
//   routes  POST /api/cash-transfers · /api/cash-transfers/{id}/submit · /dispatch · /receive · /reconcile
//   bodies  transferCreate {kind, toBranchId?, lines: [{currency, denominationId, qty: str}], note} answers
//           {id, status: DRAFT}; transferReceive lines [{currency, denominationId, qtyReceived: str}].
//   view    cash_transfers (paged, filters status, kind); rows are the design's records camelCased (#624 G1):
//           {id, kind, fromBranchId, toBranchId, fromShiftId, toShiftId, status, createdBy, note, createdAt, …}.
//           status ∈ DRAFT | APPROVED | DISPATCHED | RECEIVED | RECONCILED; kind ∈ BRANCH | DRAWER_TO_SAFE |
//           SAFE_TO_DRAWER | EXTERNAL_IN | EXTERNAL_OUT.
//   lines   one row per face value of the denominations view (#624 issuecomment-6107796305), as S-5 #578 pins: GET
//           /api/v3/views/denominations → data { rows: [{ id, currency, faceValue: string, groupCode }] }. BE ruling
//           #624 issuecomment-6107853376: the view excludes the catch-all (face_value NULL) rows, so none is shown.
//   types   commands in generated/cash.ts; cash_transfers.ts and denominations.ts for the rows (BE ruling #624
//           issuecomment-6107777140).
//
//   dest    a BRANCH transfer's destination (BE ruling #624 issuecomment-6107955194): GET
//           /api/v3/views/transfer_branches → data { rows: [{ branchId, name }] }, every branch of the session's organisation
//           except the session branch (no "active" filter: `branches` has no status column, BE #624 issuecomment-6108031658), ordered by name then branchId, authorised like transferCreate; TS in
//           generated/transfer_branches.ts. The server does not yet check the destination itself, so this list is the
//           only filter (BE security note): the picker offers exactly these rows, never the session branch.
//
// NOT PINNED HERE:
//   - receive prefilled with the quantities sent: the receive form is one row per face value, typed (FE ruling). Since
//     #624 merged, a `CashTransferRow` does carry `lines` (`qtySent`), so a prefill is possible; not pinned here.
//   - the drawer of a drawer↔safe move: #624 refuses DRAWER_TO_SAFE without `fromShiftId` and SAFE_TO_DRAWER without
//     `toShiftId` (`xc-cash/.../kinds/safe.rs:44-57`), and no view lists the branch's open shifts to this card, so the
//     create-flow tests use EXTERNAL_OUT/EXTERNAL_IN (no shifts, no destination: `kinds/external.rs:37`).
//
// The contract this test assumes ([required] unless marked):
//   transfers/card.ts  export const card: StaffCard; id = 'transfers'; visible reads only can(ctx.me, …):
//                      shown exactly with can(CashManage).
//   transfers/i18n.ts  export const th / en, same keys, no zh. Keys read here: newTransfer dispatch receive reconcile
//                      saved.
//   DOM hooks          [data-testid=transfer-new]        opens the create form
//                      [data-testid=transfer-form]       the create <form>: a <select data-testid=transfer-kind> with
//                                                        one option per kind (value = the kind), a
//                                                        <select data-testid=transfer-to-branch> shown only for BRANCH
//                                                        (one option per row, value = branchId; an empty-value
//                                                        placeholder is allowed; save disabled until one is picked),
//                                                        a note <input|textarea data-testid=transfer-note>, one
//                                                        <input data-denomination-id="<id>"> per face value, and
//                                                        [data-testid=transfer-save]
//                      [data-transfer-id="<id>"]         one element per listed transfer, holding as it applies
//                                                        [data-testid=transfer-dispatch] (APPROVED, CashManage),
//                                                        [data-testid=transfer-receive] (DISPATCHED to my branch, or a
//                                                        move inside my branch; CashManage), [data-testid=transfer-
//                                                        reconcile] (RECEIVED, FinanceReconcile)
//                      [data-testid=transfer-receive-form] the receive <form> with one
//                                                        <input data-denomination-id> per face value and
//                                                        [data-testid=transfer-receive-save]
//                      the reconcile note is entered in parts/ReasonDialog (its <input>, then the staff confirm
//                      button); cancel sends nothing.
//                      [data-testid=transfer-notice] / [data-testid=transfer-error]
//   Bodies             a `branch` member, if sent, must be the actor's own branch; nothing else beyond the design's.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { errorText, setLang } from "../../../shared/i18n";
import {
  makeCashTransferRow,
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffMeView,
  makeTransferBranchRow,
  type StaffFloorView,
  type StaffMeView,
} from "../../../shared/testing/fixtures";
import type { StaffCard, StaffCtx } from "../cards";
import { th as staffTh } from "../i18n";
import { warmModules } from "../../../shared/testing/warm-modules";

type Texts = {
  th: Record<string, string>;
  en: Record<string, string>;
} & Record<string, unknown>;

const cardMods = import.meta.glob<{ card: StaffCard }>("./card.ts");
const textMods = import.meta.glob<Texts>("./i18n.ts", { eager: true });
const ownSources = import.meta.glob<string>(
  ["./*.ts", "./*.tsx", "!./*.test.ts", "!./*.test.tsx"],
  {
    query: "?raw",
    import: "default",
    eager: true,
  },
);
const staffSources = import.meta.glob<string>(["../**/*.ts", "../**/*.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
});

async function loadCard(): Promise<StaffCard> {
  const l = cardMods["./card.ts"];
  expect(l, "web/src/screens/staff/transfers/card.ts exists").toBeDefined();
  return (await l!()).card;
}

function texts(): Texts {
  const m = textMods["./i18n.ts"];
  expect(m, "web/src/screens/staff/transfers/i18n.ts exists").toBeDefined();
  return m!;
}

const KEYS = [
  "newTransfer",
  "dispatch",
  "receive",
  "reconcile",
  "saved",
] as const;

function th(key: (typeof KEYS)[number]): string {
  const v = texts().th[key];
  expect(v, `transfers th['${key}']`).toBeTypeOf("string");
  return v!;
}

// ── fixtures ────────────────────────────────────────────────────────────────────────────────

const BRANCH = "MAIN";
const ELSEWHERE = "B09";
const ME = "00000000-0000-4000-8000-0000000000b1";
const KINDS = [
  "BRANCH",
  "DRAWER_TO_SAFE",
  "SAFE_TO_DRAWER",
  "EXTERNAL_IN",
  "EXTERNAL_OUT",
];

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

function floor() {
  return makeStaffFloorView({
    phase: "OPEN",
    hours: null,
    myClockIn: "2026-10-11T01:00:00Z",
    myShift: null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

const DENOMS = [
  makeDenominationRow({ id: 12, currency: "THB", faceValue: "100.0000", groupCode: "NOTE" }),
  makeDenominationRow({ id: 11, currency: "THB", faceValue: "1000.0000", groupCode: "NOTE" }),
  makeDenominationRow({ id: 21, currency: "USD", faceValue: "50.0000", groupCode: "NOTE" }),
];

function transfer(
  id: string,
  status: string,
  kind = "BRANCH",
  from = BRANCH,
  to: string | null = ELSEWHERE,
) {
  return makeCashTransferRow({
    id,
    kind,
    fromBranchId: from,
    toBranchId: to,
    fromShiftId: null,
    toShiftId: null,
    status,
    createdBy: "00000000-0000-4000-8000-0000000000b2",
    note: "n",
    createdAt: "2026-10-11T02:00:00Z",
    lines: [],
  });
}

const T_DRAFT = "e0000000-0000-4000-8000-000000000001";
const T_APPROVED = "e0000000-0000-4000-8000-000000000002";
const T_IN = "e0000000-0000-4000-8000-000000000003"; // dispatched from B09 to my branch
const T_OUT = "e0000000-0000-4000-8000-000000000004"; // dispatched from my branch to B09
const T_RECEIVED = "e0000000-0000-4000-8000-000000000005";
const T_DONE = "e0000000-0000-4000-8000-000000000006";
const T_SAFE = "e0000000-0000-4000-8000-000000000007"; // a drawer-to-safe move inside my branch, dispatched

const TRANSFERS = [
  transfer(T_DRAFT, "DRAFT"),
  transfer(T_APPROVED, "APPROVED"),
  transfer(T_IN, "DISPATCHED", "BRANCH", ELSEWHERE, BRANCH),
  transfer(T_OUT, "DISPATCHED", "BRANCH", BRANCH, ELSEWHERE),
  transfer(T_RECEIVED, "RECEIVED", "BRANCH", ELSEWHERE, BRANCH),
  transfer(T_DONE, "RECONCILED"),
  transfer(T_SAFE, "DISPATCHED", "DRAWER_TO_SAFE", BRANCH, null),
];

const NEW_ID = "f0000000-0000-4000-8000-0000000000aa";

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
    cash_transfers: TRANSFERS,
    denominations: { rows: DENOMS },
    transfer_branches: { rows: [] },
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
      if (path === "/api/cash-transfers")
        return json(200, { id: NEW_ID, status: "DRAFT" });
      return json(200, { id: path.split("/")[3], status: "DRAFT" });
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

async function mount(m: StaffMeView) {
  const card = await loadCard();
  const { ctx, refresh } = ctxOf(m, floor());
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
  return { refresh };
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

function inRow(id: string, control: string): HTMLElement | null {
  return q(`[data-transfer-id="${id}"] ${tid(control)}`);
}

async function click(el: Element) {
  (el as HTMLElement).click();
  await settle();
}

function typeInto(
  input: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement,
  value: string,
) {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

/**
 * The body a command sent, compared on what it says: #624's generated inputs (`cash.ts`) carry `branch: string | null`
 * and optional members as `null` (`toBranchId`, `fromShiftId`, `toShiftId`), so a `branch` member (the actor's own,
 * or null) and every null-valued member are left out before comparing. Merging main after #624 (2026-10-11).
 */
function bodyOf(s: Sent): Record<string, unknown> {
  expect(s.body, `${s.url} has a JSON body`).toBeTypeOf("string");
  const b = JSON.parse(s.body!) as Record<string, unknown>;
  if ("branch" in b) {
    expect(
      [BRANCH, null],
      "a branch member names the actor's own branch, or null",
    ).toContain(b["branch"]);
    delete b["branch"];
  }
  for (const k of Object.keys(b)) if (b[k] === null) delete b[k];
  return b;
}

function linesOf(body: Record<string, unknown>): unknown[] {
  return [...(body["lines"] as { denominationId: number }[])].sort(
    (a, b) => a.denominationId - b.denominationId,
  );
}

function qtyInput(form: Element, id: number): HTMLInputElement {
  const i = form.querySelector<HTMLInputElement>(
    `input[data-denomination-id="${id}"]`,
  );
  expect(i, `a quantity input for denomination ${id}`).not.toBeNull();
  return i!;
}

async function openCreate(): Promise<HTMLFormElement> {
  await click(must(tid("transfer-new")));
  await vi.waitFor(() => expect(q(tid("transfer-form"))).not.toBeNull());
  const form = must<HTMLFormElement>(tid("transfer-form"));
  expect(form.tagName).toBe("FORM");
  await vi.waitFor(() =>
    expect(form.querySelector("input[data-denomination-id]")).not.toBeNull(),
  );
  return form;
}

async function fillMove(form: HTMLFormElement, kind: string, note: string) {
  typeInto(must<HTMLSelectElement>(tid("transfer-kind")), kind);
  await settle();
  typeInto(must<HTMLInputElement>(tid("transfer-note")), note);
  typeInto(qtyInput(form, 21), "2");
}

async function save(sel: string) {
  must<HTMLButtonElement>(tid(sel)).click();
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
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  setLang("th");
});

// ── the card record and its texts ───────────────────────────────────────────────────────────

describe("S-6 the card record (SD:§4)", () => {
  it('exports card { id: "transfers", order, visible, Component }', async () => {
    const card = await loadCard();
    expect(card.id).toBe("transfers");
    expect(Number.isFinite(card.order)).toBe(true);
    expect(card.visible).toBeTypeOf("function");
    expect(card.Component).toBeTypeOf("function");
  });

  it("transfers/i18n.ts has th and en with the same keys, and no zh table (S8)", () => {
    const t = texts();
    expect(Object.keys(t.en).sort()).toEqual(Object.keys(t.th).sort());
    for (const k of KEYS) {
      expect(t.th[k], `th['${k}']`).toBeTypeOf("string");
      expect(t.en[k], `en['${k}']`).toBeTypeOf("string");
      expect(t.en[k], `en['${k}'] is English`).not.toBe(t.th[k]);
    }
    expect(th("saved")).toBe("บันทึกแล้ว");
    expect("zh" in t, "staff screens are TH/EN only").toBe(false);
  });

  // The generated files are imported above as types (CA-8 #624 merged): a missing or renamed one is a type error.
  it("the card imports cash.ts, cash_transfers.ts and denominations.ts", () => {
    const code = Object.values(ownSources).join("\n");
    expect(
      Object.keys(ownSources).length,
      "transfers/ has code besides its tests",
    ).toBeGreaterThan(0);
    for (const f of [
      "cash",
      "cash_transfers",
      "denominations",
      "transfer_branches",
    ]) {
      expect(
        new RegExp(
          `from\\s+['"](\\.\\./){3}shared/api/generated/${f}['"]`,
        ).test(code),
        `a transfers/ source imports ../../../shared/api/generated/${f}`,
      ).toBe(true);
    }
  });
});

// ── B8: who sees what ───────────────────────────────────────────────────────────────────────

describe("B8 the card and its controls by permission", () => {
  it("visible exactly when the actor holds CashManage", async () => {
    const card = await loadCard();
    const v = (perms: string[]) => card.visible(ctxOf(me(perms), floor()).ctx);
    expect(v(["CashManage"])).toBe(true);
    expect(v(["CashManage", "FinanceReconcile"])).toBe(true);
    expect(v([])).toBe(false);
    expect(
      v(["ShiftOperate", "AttendanceClock", "BranchDayOperate", "CompatRead"]),
    ).toBe(false);
  });

  it("lists the branch's transfers, one element per row", async () => {
    stub();
    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(q(`[data-transfer-id="${T_APPROVED}"]`)).not.toBeNull(),
    );
    for (const t of TRANSFERS)
      expect(q(`[data-transfer-id="${t.id}"]`), t.id).not.toBeNull();
  });

  it("dispatch on APPROVED only; receive on DISPATCHED to my branch or inside it, not one sent away", async () => {
    stub();
    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED, "transfer-dispatch")).not.toBeNull(),
    );
    expect(inRow(T_APPROVED, "transfer-dispatch")!.textContent).toContain(
      th("dispatch"),
    );
    expect(inRow(T_IN, "transfer-receive")!.textContent).toContain(
      th("receive"),
    );
    expect(
      inRow(T_SAFE, "transfer-receive"),
      "a move inside my branch is received here",
    ).not.toBeNull();
    expect(
      inRow(T_OUT, "transfer-receive"),
      "one dispatched to another branch is received there",
    ).toBeNull();
    for (const id of [T_DRAFT, T_IN, T_OUT, T_RECEIVED, T_DONE, T_SAFE]) {
      expect(inRow(id, "transfer-dispatch"), `${id}: no dispatch`).toBeNull();
    }
    for (const id of [T_DRAFT, T_APPROVED, T_RECEIVED, T_DONE]) {
      expect(inRow(id, "transfer-receive"), `${id}: no receive`).toBeNull();
    }
  });

  it("reconcile on RECEIVED only, and only with FinanceReconcile", async () => {
    stub();
    await mount(me(["CashManage", "FinanceReconcile"]));
    await vi.waitFor(() =>
      expect(inRow(T_RECEIVED, "transfer-reconcile")).not.toBeNull(),
    );
    expect(inRow(T_RECEIVED, "transfer-reconcile")!.textContent).toContain(
      th("reconcile"),
    );
    for (const id of [T_DRAFT, T_APPROVED, T_IN, T_OUT, T_DONE, T_SAFE]) {
      expect(inRow(id, "transfer-reconcile"), `${id}: no reconcile`).toBeNull();
    }
    dispose?.();
    dispose = undefined;
    host.innerHTML = "";
    stub();
    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(q(`[data-transfer-id="${T_RECEIVED}"]`)).not.toBeNull(),
    );
    expect(
      inRow(T_RECEIVED, "transfer-reconcile"),
      "without FinanceReconcile",
    ).toBeNull();
  });

  it("the create form offers every kind, with the destination branch only for BRANCH (#582 C5: one flow)", async () => {
    stub();
    await mount(me(["CashManage"]));
    expect(must(tid("transfer-new")).textContent).toContain(th("newTransfer"));
    await openCreate();
    const kinds = [
      ...must<HTMLSelectElement>(tid("transfer-kind")).options,
    ].map((o) => o.value);
    expect([...kinds].sort()).toEqual([...KINDS].sort());
    typeInto(must<HTMLSelectElement>(tid("transfer-kind")), "BRANCH");
    await settle();
    expect(
      q(tid("transfer-to-branch")),
      "BRANCH asks for the destination",
    ).not.toBeNull();
    for (const k of KINDS.filter((x) => x !== "BRANCH")) {
      typeInto(must<HTMLSelectElement>(tid("transfer-kind")), k);
      await settle();
      expect(
        q(tid("transfer-to-branch")),
        `${k} has no destination branch`,
      ).toBeNull();
    }
  });

  it("the form's lines are one row per face value, each labelled with its faceValue exactly as the wire string", async () => {
    stub();
    await mount(me(["CashManage"]));
    const form = await openCreate();
    for (const d of DENOMS) {
      const input = qtyInput(form, d.id);
      const row =
        input.closest("[data-denomination-row]") ?? input.parentElement!;
      expect(row.textContent).toContain(d.faceValue);
      expect(row.textContent).toContain(d.currency);
    }
  });
});

// ── C8 / C9: what each control sends ────────────────────────────────────────────────────────

describe("C8 a new move is transferCreate then transferSubmit", () => {
  it("EXTERNAL_OUT: POST /api/cash-transfers {kind, lines, note}, then /{id}/submit {id} with the answered id", async () => {
    stub();
    await mount(me(["CashManage"]));
    const form = await openCreate();
    await fillMove(form, "EXTERNAL_OUT", "to the bank");
    await save("transfer-save");
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      "POST /api/cash-transfers",
      `POST /api/cash-transfers/${NEW_ID}/submit`,
    ]);
    const create = bodyOf(sent[0]!);
    expect(Object.keys(create).sort()).toEqual(["kind", "lines", "note"]);
    expect(create["kind"]).toBe("EXTERNAL_OUT");
    expect(create["note"]).toBe("to the bank");
    expect(linesOf(create)).toEqual([
      { currency: "USD", denominationId: 21, qty: "2" },
    ]);
    expect(bodyOf(sent[1]!)).toEqual({ id: NEW_ID });
    expect(sent[0]!.key).toBeTruthy();
    expect(sent[1]!.key).toBeTruthy();
    expect(sent[1]!.key, "two commands, two keys").not.toBe(sent[0]!.key);
  });

  it("EXTERNAL_IN goes through the same flow and controls", async () => {
    stub();
    await mount(me(["CashManage"]));
    const form = await openCreate();
    await fillMove(form, "EXTERNAL_IN", "from the bank");
    await save("transfer-save");
    expect(sent.map((s) => s.url)).toEqual([
      "/api/cash-transfers",
      `/api/cash-transfers/${NEW_ID}/submit`,
    ]);
    expect(bodyOf(sent[0]!)["kind"]).toBe("EXTERNAL_IN");
  });

  it("if submit fails on the network, the retry resends only the submit, with its key and body unchanged", async () => {
    stub();
    answers[`/api/cash-transfers/${NEW_ID}/submit`] = [
      "network",
      { status: 200, body: { id: NEW_ID, status: "DRAFT" } },
    ];
    await mount(me(["CashManage"]));
    const form = await openCreate();
    await fillMove(form, "EXTERNAL_OUT", "to the bank");
    await save("transfer-save");
    await save("transfer-save");
    expect(sent.map((s) => s.url)).toEqual([
      "/api/cash-transfers",
      `/api/cash-transfers/${NEW_ID}/submit`,
      `/api/cash-transfers/${NEW_ID}/submit`,
    ]);
    expect(sent[2]!.key).toBe(sent[1]!.key);
    expect(sent[2]!.body).toBe(sent[1]!.body);
  });

  it("if create fails on the network, the retry resends the create with its key and a byte-equal body", async () => {
    stub();
    answers["/api/cash-transfers"] = [
      "network",
      { status: 200, body: { id: NEW_ID, status: "DRAFT" } },
    ];
    await mount(me(["CashManage"]));
    const form = await openCreate();
    await fillMove(form, "EXTERNAL_OUT", "to the bank");
    await save("transfer-save");
    expect(sent.map((s) => s.url)).toEqual(["/api/cash-transfers"]);
    await save("transfer-save");
    expect(sent[1]!.url).toBe("/api/cash-transfers");
    expect(sent[1]!.key).toBe(sent[0]!.key);
    expect(sent[1]!.body).toBe(sent[0]!.body);
  });

  it("a refused create shows t('error.'+code), re-reads nothing, and the form keeps its values", async () => {
    stub();
    answers["/api/cash-transfers"] = [
      {
        status: 403,
        body: { error: { code: "roleNotAllowedHere", message: "refused" } },
      },
    ];
    const { refresh } = await mount(me(["CashManage"]));
    const form = await openCreate();
    await fillMove(form, "EXTERNAL_OUT", "to the bank");
    const before = reads.length;
    await save("transfer-save");
    expect(must(tid("transfer-error")).textContent).toBe(
      errorText("roleNotAllowedHere", "refused"),
    );
    expect(refresh).not.toHaveBeenCalled();
    expect(reads.length).toBe(before);
    expect(
      sent.map((s) => s.url),
      "no submit after a refused create",
    ).toEqual(["/api/cash-transfers"]);
    expect(must<HTMLInputElement>(tid("transfer-note")).value).toBe(
      "to the bank",
    );
    expect(qtyInput(must(tid("transfer-form")), 21).value).toBe("2");
    expect(must<HTMLSelectElement>(tid("transfer-kind")).value).toBe(
      "EXTERNAL_OUT",
    );
  });
});

// ── C8 a BRANCH transfer's destination (transfer_branches, #624 issuecomment-6107955194) ─────────────

/** transfer_branches as the server orders them (name, then branchId); the session branch is in it only to prove it is never offered. */
const DESTINATIONS = [
  makeTransferBranchRow({ branchId: ELSEWHERE, name: "Ayutthaya" }),
  makeTransferBranchRow({ branchId: BRANCH, name: "Main" }),
  makeTransferBranchRow({ branchId: "B07", name: "Phuket" }),
];

function destinationIds(): string[] {
  return [...must<HTMLSelectElement>(tid("transfer-to-branch")).options]
    .map((o) => o.value)
    .filter((v) => v !== "");
}

async function chooseBranchKind(): Promise<HTMLFormElement> {
  const form = await openCreate();
  typeInto(must<HTMLSelectElement>(tid("transfer-kind")), "BRANCH");
  await settle();
  return form;
}

describe("C8 a BRANCH transfer's destination comes from transfer_branches, the only filter (BE security note)", () => {
  it("the picker lists exactly the rows in the server's order, never the session branch; toBranchId is the picked branchId", async () => {
    stub({ transfer_branches: { rows: DESTINATIONS } });
    await mount(me(["CashManage"]));
    const form = await chooseBranchKind();
    await vi.waitFor(() => expect(destinationIds().length).toBeGreaterThan(0));
    const select = must<HTMLSelectElement>(tid("transfer-to-branch"));
    expect(select.tagName).toBe("SELECT");
    expect(
      destinationIds(),
      "exactly the rows, in order, without the session branch",
    ).toEqual([ELSEWHERE, "B07"]);
    expect(
      [...select.options].find((o) => o.value === ELSEWHERE)!.textContent,
    ).toContain("Ayutthaya");
    typeInto(select, ELSEWHERE);
    typeInto(must<HTMLInputElement>(tid("transfer-note")), "to Ayutthaya");
    typeInto(qtyInput(form, 21), "2");
    await settle();
    await save("transfer-save");
    expect(sent.map((s) => s.url)).toEqual([
      "/api/cash-transfers",
      `/api/cash-transfers/${NEW_ID}/submit`,
    ]);
    const create = bodyOf(sent[0]!);
    expect(Object.keys(create).sort()).toEqual([
      "kind",
      "lines",
      "note",
      "toBranchId",
    ]);
    expect(create["kind"]).toBe("BRANCH");
    expect(create["toBranchId"]).toBe(ELSEWHERE);
  });

  it("the session branch is never offered, even when it is the only row", async () => {
    stub({ transfer_branches: { rows: [{ branchId: BRANCH, name: "Main" }] } });
    await mount(me(["CashManage"]));
    await chooseBranchKind();
    await settle();
    expect(destinationIds()).toEqual([]);
  });

  it("zero rows: no BRANCH transfer can be submitted", async () => {
    stub({ transfer_branches: { rows: [] } });
    await mount(me(["CashManage"]));
    const form = await chooseBranchKind();
    typeInto(must<HTMLInputElement>(tid("transfer-note")), "nowhere");
    typeInto(qtyInput(form, 21), "2");
    await settle();
    expect(destinationIds()).toEqual([]);
    const button = must<HTMLButtonElement>(tid("transfer-save"));
    expect(button.disabled, "save is disabled with no destination").toBe(true);
    await save("transfer-save");
    expect(sent, "nothing is sent").toEqual([]);
  });

  it("a BRANCH transfer with no destination picked cannot be submitted either", async () => {
    stub({ transfer_branches: { rows: DESTINATIONS } });
    await mount(me(["CashManage"]));
    const form = await chooseBranchKind();
    await vi.waitFor(() => expect(destinationIds().length).toBeGreaterThan(0));
    typeInto(qtyInput(form, 21), "2");
    await settle();
    expect(must<HTMLButtonElement>(tid("transfer-save")).disabled).toBe(true);
    await save("transfer-save");
    expect(sent).toEqual([]);
  });

  it("every other kind sends no toBranchId, even after a destination was picked under BRANCH", async () => {
    for (const kind of KINDS.filter((k) => k !== "BRANCH")) {
      sent = [];
      stub({ transfer_branches: { rows: DESTINATIONS } });
      await mount(me(["CashManage"]));
      const form = await chooseBranchKind();
      await vi.waitFor(() =>
        expect(destinationIds().length).toBeGreaterThan(0),
      );
      typeInto(must<HTMLSelectElement>(tid("transfer-to-branch")), ELSEWHERE);
      await settle();
      await fillMove(form, kind, `a ${kind} move`);
      await save("transfer-save");
      const create = bodyOf(sent[0]!);
      expect(create["kind"], kind).toBe(kind);
      expect("toBranchId" in create, `${kind} sends no toBranchId`).toBe(false);
      dispose?.();
      dispose = undefined;
      host.innerHTML = "";
    }
  });
});

describe("C9 dispatch, receive and reconcile send exactly their command and body", () => {
  it("dispatch sends POST /api/cash-transfers/{id}/dispatch {id}", async () => {
    stub();
    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED, "transfer-dispatch")).not.toBeNull(),
    );
    await click(inRow(T_APPROVED, "transfer-dispatch")!);
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      `POST /api/cash-transfers/${T_APPROVED}/dispatch`,
    ]);
    expect(bodyOf(sent[0]!)).toEqual({ id: T_APPROVED });
  });

  it("receive sends /{id}/receive {id, lines: [{currency, denominationId, qtyReceived: str}]} from the typed rows", async () => {
    stub();
    await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_IN, "transfer-receive")).not.toBeNull(),
    );
    await click(inRow(T_IN, "transfer-receive")!);
    await vi.waitFor(() =>
      expect(
        q(`${tid("transfer-receive-form")} input[data-denomination-id]`),
      ).not.toBeNull(),
    );
    const form = must<HTMLFormElement>(tid("transfer-receive-form"));
    typeInto(qtyInput(form, 21), "2");
    typeInto(qtyInput(form, 11), "1");
    await save("transfer-receive-save");
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      `POST /api/cash-transfers/${T_IN}/receive`,
    ]);
    const body = bodyOf(sent[0]!);
    expect(Object.keys(body).sort()).toEqual(["id", "lines"]);
    expect(body["id"]).toBe(T_IN);
    expect(linesOf(body)).toEqual([
      { currency: "THB", denominationId: 11, qtyReceived: "1" },
      { currency: "USD", denominationId: 21, qtyReceived: "2" },
    ]);
    expect(sent[0]!.body).not.toMatch(/"qtyReceived":\s*\d/);
  });

  it("reconcile asks for the note in ReasonDialog; confirm sends /{id}/reconcile {id, note}, cancel sends nothing", async () => {
    stub();
    await mount(me(["CashManage", "FinanceReconcile"]));
    await vi.waitFor(() =>
      expect(inRow(T_RECEIVED, "transfer-reconcile")).not.toBeNull(),
    );
    await click(inRow(T_RECEIVED, "transfer-reconcile")!);
    let dialog = must("dialog");
    expect(sent).toEqual([]);
    await click(
      [...dialog.querySelectorAll("button")].find(
        (b) => b.textContent === staffTh.cancel,
      )!,
    );
    expect(sent).toEqual([]);
    await click(inRow(T_RECEIVED, "transfer-reconcile")!);
    dialog = must("dialog");
    typeInto(dialog.querySelector("input")!, "checked");
    await click(
      [...dialog.querySelectorAll("button")].find(
        (b) => b.textContent === staffTh.confirm,
      )!,
    );
    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      `POST /api/cash-transfers/${T_RECEIVED}/reconcile`,
    ]);
    expect(bodyOf(sent[0]!)).toEqual({ id: T_RECEIVED, note: "checked" });
  });
});

// ── after a write ───────────────────────────────────────────────────────────────────────────

describe("SD §6.2 after a write", () => {
  it("success: the notice is t(saved), staff_floor is re-read (ctx.refresh) and cash_transfers is re-read", async () => {
    stub();
    const { refresh } = await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED, "transfer-dispatch")).not.toBeNull(),
    );
    const before = reads.filter((r) => r === "cash_transfers").length;
    await click(inRow(T_APPROVED, "transfer-dispatch")!);
    await vi.waitFor(() =>
      expect(must(tid("transfer-notice")).textContent).toBe(th("saved")),
    );
    expect(refresh).toHaveBeenCalledTimes(1);
    expect(reads.filter((r) => r === "cash_transfers").length).toBeGreaterThan(
      before,
    );
  });

  it("a refused dispatch shows t('error.'+code) and re-reads nothing", async () => {
    stub();
    answers[`/api/cash-transfers/${T_APPROVED}/dispatch`] = [
      {
        status: 409,
        body: { error: { code: "roleNotAllowedHere", message: "refused" } },
      },
    ];
    const { refresh } = await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED, "transfer-dispatch")).not.toBeNull(),
    );
    const before = reads.length;
    await click(inRow(T_APPROVED, "transfer-dispatch")!);
    expect(must(tid("transfer-error")).textContent).toBe(
      errorText("roleNotAllowedHere", "refused"),
    );
    expect(refresh).not.toHaveBeenCalled();
    expect(reads.length).toBe(before);
  });

  it("a retried dispatch after a network error resends the same key and a byte-equal body", async () => {
    stub();
    answers[`/api/cash-transfers/${T_APPROVED}/dispatch`] = [
      "network",
      { status: 200, body: { id: T_APPROVED } },
    ];
    const { refresh } = await mount(me(["CashManage"]));
    await vi.waitFor(() =>
      expect(inRow(T_APPROVED, "transfer-dispatch")).not.toBeNull(),
    );
    await click(inRow(T_APPROVED, "transfer-dispatch")!);
    await click(inRow(T_APPROVED, "transfer-dispatch")!);
    expect(sent).toHaveLength(2);
    expect(sent[1]!.key).toBe(sent[0]!.key);
    expect(sent[1]!.body).toBe(sent[0]!.body);
    expect(refresh).toHaveBeenCalledTimes(1);
  });
});

// ── the real page ───────────────────────────────────────────────────────────────────────────

describe("the real /staff page mounts the transfers card through main.tsx and cards.ts", () => {
  it('main.tsx, with the ./*/card.ts glob, renders data-card="transfers"', async () => {
    const app = document.createElement("div");
    app.id = "app";
    document.body.appendChild(app);
    stub({
      staff_me: me(["CashManage"]),
      staff_floor: floor(),
      me: {
        kind: "user",
        id: ME,
        role: "any",
        branch: BRANCH,
        name: "Ploy",
        username: "ploy",
        branchName: "XC-B",
        branches: [{ id: BRANCH, code: "XC-B", name: "XC-B" }],
      },
    });
    vi.resetModules();
    await import("../main");
    await vi.waitFor(() =>
      expect(app.querySelector('[data-card="transfers"]')).not.toBeNull(),
    );
    expect(app.querySelectorAll('[data-card="transfers"]').length).toBe(1);
    expect(
      app.querySelector(`[data-card="transfers"] ${tid("transfer-new")}`),
    ).not.toBeNull();
  });
});

// ── the folder's static rules ───────────────────────────────────────────────────────────────

describe("SD §8.1 money stays a string in web/src/screens/staff/**", () => {
  it("no Number-constructor call in any staff source (tests included)", () => {
    const pattern = new RegExp("\\b" + "Num" + "ber\\(");
    const hits: string[] = [];
    for (const [file, src] of Object.entries(staffSources)) {
      src.split("\n").forEach((line, i) => {
        if (pattern.test(line)) hits.push(`${file}:${i + 1}: ${line.trim()}`);
      });
    }
    expect(
      Object.keys(staffSources),
      "the scan reaches transfers/ (its own files key as ./)",
    ).toContain("./card.ts");
    expect(Object.keys(staffSources)).toContain("../cards.ts");
    expect(hits).toEqual([]);
  });

  it("no transfers/ source writes through /api/staff/rpc (S5)", () => {
    for (const [file, src] of Object.entries(ownSources)) {
      expect(src, file).not.toContain("/api/staff/rpc");
    }
  });
});
