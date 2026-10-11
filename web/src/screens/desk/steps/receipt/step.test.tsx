// #543 Part 2, DK-H: step 4, save result and receipt (design docs/design/xchange-v3-wave3-desk.md D-2, D-3, §3.2 rows
// "4 Save", "4 Result box", "4 Receipt", "4 after save", §3.6b row "Step 4", §3.7, §3.9 DK-1 rows, §5.4, §6 row
// ReceiptStep). Every test runs on both OWNER_DESK and STAFF_DESK (§10). Written before the implementation by the
// test-writer (claude/709ec749, on the FE lane controller claude/5edf7bb5's order). Do not edit, delete or weaken these
// tests; add your own elsewhere.
//
// [required] by these tests (each module is reached through import.meta.glob, so a missing file fails an assertion):
// - `steps/receipt/step.tsx` exports `step: DeskStep` with `index: 4` and `component({ store })`.
// - `steps/receipt/i18n.ts` exports `th` and `en` with the same keys; Thai is v2's copy verbatim, or §3.7's v3 wording.
// - It reads `store.outcome`, `store.saved` (`{ id, status, documentRef }`, `save.ts` `SaveAnswer`), `store.saveError`,
//   `store.path.devicesLink`, `store.branch`, `store.config().adminUrl`, and calls `submit()`, `backToEdit()`,
//   `startNew()`.
// - D-2 (owner 2026-10-09): nothing receipt-like before `outcome() === 'saved'`: no iframe and no document number.
// - DK-1: `refused` → `← กลับไปแก้ไขรายการ` (backToEdit) and New enabled; `unknown` → the note beside `ลองบันทึกซ้ำ`
//   (submit), New and Print disabled; `saved` → New and Print enabled.
// NOT pinned here, on purpose: what Print prints and how. O-2 (the stored receipt's `@page`) is open, the stored receipt
// is read through TD-2's `receipt_document` (#1294) and printed through TD-3's shared `ReceiptFrame` (#1291, design
// `desk-transaction-detail.md` D5), and neither has merged. The tests pin only when Print is enabled.
// The store is a stub with §5.3's names, so this file does not depend on `desk-store.ts`.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { setLang } from "../../../../shared/i18n";

type StepModule = { step: { index: number; label: () => string; component: (p: { store: any }) => JSX.Element } };
type TextsModule = { th: Record<string, string>; en: Record<string, string> };
const stepMods = import.meta.glob<StepModule>("./step.tsx");
const textMods = import.meta.glob<TextsModule>("./i18n.ts");
const SOURCES = import.meta.glob<string>(["./*.ts", "./*.tsx", "!./*.test.ts", "!./*.test.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
});

async function load<T>(mods: Record<string, () => Promise<T>>, key: string): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/desk/steps/receipt/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

const HEADING = "ใบเสร็จรายการแลกเงิน";
const NEW = "ทำรายการใหม่";
const PRINT = "พิมพ์ใบเสร็จ NEOCAL 80 มม.";
const RETRY = "ลองบันทึกซ้ำ";
const BACK_EDIT = "← กลับไปแก้ไขรายการ";
const SAVING = "กำลังบันทึกรายการและไฟล์แนบ…";
const UNKNOWN_NOTE = 'ยังไม่ทราบผลการบันทึก กด "ลองบันทึกซ้ำ" ได้ ระบบไม่บันทึกรายการเดียวกันซ้ำ';
const ADMIN_LINK = "เปิดรายการในหน้าแอดมิน";
const DEVICES = "ดูงานพิมพ์และบันทึกหลังบ้าน →";
const DOC = "XC-20261011-000001";

const PATHS = {
  OWNER_DESK: { kind: "owner", devicesLink: true },
  STAFF_DESK: { kind: "staff", devicesLink: false },
} as const;

type Outcome = "idle" | "saving" | "saved" | "refused" | "unknown";

function stubStore(path: { devicesLink: boolean }, init: { outcome: Outcome; saveError?: string }) {
  const [outcome, setOutcome] = createSignal<Outcome>(init.outcome);
  const [step] = createSignal(4);
  return {
    path,
    step,
    branch: () => "MAIN",
    config: () => ({ shiftOpen: true, adminUrl: "/?branch=MAIN", paymentMethods: [] }),
    outcome,
    transactionState: () => (init.outcome === "saved" ? "COMPLETED" : undefined),
    saveError: () => init.saveError,
    saved: () => (outcome() === "saved" ? { id: "tx-1", status: "COMPLETED", documentRef: DOC } : undefined),
    submit: vi.fn(async () => "saved"),
    backToEdit: vi.fn(),
    startNew: vi.fn(() => true),
    go: vi.fn(),
    _setOutcome: setOutcome,
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => {
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
  vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("step 4 must not call the network itself"))));
});
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang("th");
});

async function mount(store: ReturnType<typeof stubStore>) {
  const { step } = await load(stepMods, "./step.tsx");
  dispose = render(() => step.component({ store }), host);
  return step;
}

const button = (text: string) =>
  [...host.querySelectorAll<HTMLButtonElement>("button")].find((b) => (b.textContent ?? "").trim() === text);
const link = (text: string) =>
  [...host.querySelectorAll<HTMLAnchorElement>("a")].find((a) => (a.textContent ?? "").trim() === text);

function expectNoReceipt() {
  expect(host.querySelector("iframe"), "no receipt frame before the save (D-2)").toBeNull();
  expect(host.textContent, "no document number before the save (D-2)").not.toContain(DOC);
}

describe("steps/receipt: the module", () => {
  it("H0 exports step 4 with a label from its own i18n table", async () => {
    const { step } = await load(stepMods, "./step.tsx");
    expect(step.index).toBe(4);
    expect(step.label().trim()).not.toBe("");
  });

  it("H0b i18n.ts has th and en with the same keys; th carries v2's step-4 copy and §3.7's rewrite", async () => {
    const { th, en } = await load(textMods, "./i18n.ts");
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    expect((th as Record<string, unknown>).zh).toBeUndefined();
    const values = Object.values(th);
    for (const s of [HEADING, NEW, PRINT, RETRY, BACK_EDIT, SAVING, UNKNOWN_NOTE, ADMIN_LINK, DEVICES]) {
      expect(values, s).toContain(s);
    }
    // D-3: v2's Google Sheets wording is not carried over
    expect(values.join("\n")).not.toMatch(/Sheet|เปิดแถวรายการ|PENDING SAVE|SAMPLE \/ DEMO/);
  });

  it("H0c no money arithmetic and no fetch( in the step's sources (W7)", () => {
    expect(Object.keys(SOURCES)).toContain("./step.tsx");
    for (const [file, src] of Object.entries(SOURCES)) {
      expect(src, file).not.toMatch(/Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100/);
      expect(src, file).not.toMatch(/\bfetch\(/);
    }
  });
});

for (const which of ["OWNER_DESK", "STAFF_DESK"] as const) {
  describe(`steps/receipt on ${which}`, () => {
    it("H1 saving: §3.7's saving text; no receipt; New and Print disabled", async () => {
      const store = stubStore(PATHS[which], { outcome: "saving" });
      await mount(store);
      expect(host.textContent).toContain(HEADING);
      expect(host.textContent).toContain(SAVING);
      expectNoReceipt();
      expect(button(NEW)?.disabled ?? true).toBe(true);
      expect(button(PRINT)?.disabled ?? true).toBe(true);
    });

    it("H2 saved: the server's document number, New and Print enabled, the admin link (§3.7)", async () => {
      const store = stubStore(PATHS[which], { outcome: "saved" });
      await mount(store);
      expect(host.textContent).toContain(DOC);
      expect(button(NEW)?.disabled).toBe(false);
      expect(button(PRINT)?.disabled).toBe(false);
      const a = link(ADMIN_LINK);
      expect(a, ADMIN_LINK).toBeDefined();
      expect(a!.getAttribute("href")).toBe("/?branch=MAIN#transactions");
    });

    it(`H3 the devices link after a save is ${which === "OWNER_DESK" ? "shown" : "absent"} (desk-devices.js:5, path.devicesLink)`, async () => {
      const store = stubStore(PATHS[which], { outcome: "saved" });
      await mount(store);
      const a = link(DEVICES);
      if (which === "OWNER_DESK") {
        expect(a, DEVICES).toBeDefined();
        expect(a!.getAttribute("href")).toBe("/devices?branch=MAIN");
      } else {
        expect(a).toBeUndefined();
      }
    });

    it("H4 refused: the server's sentence, ← กลับไปแก้ไขรายการ calls backToEdit, New enabled, Print disabled, no receipt", async () => {
      const store = stubStore(PATHS[which], { outcome: "refused", saveError: "ใบเสนอราคาหมดอายุ" });
      await mount(store);
      expect(host.textContent).toContain("ใบเสนอราคาหมดอายุ");
      expectNoReceipt();
      expect(button(NEW)?.disabled).toBe(false);
      expect(button(PRINT)?.disabled ?? true).toBe(true);
      button(BACK_EDIT)!.click();
      expect(store.backToEdit).toHaveBeenCalledTimes(1);
    });

    it("H5 unknown: the note beside ลองบันทึกซ้ำ, which calls submit; New and Print disabled; no receipt", async () => {
      const store = stubStore(PATHS[which], { outcome: "unknown", saveError: "network" });
      await mount(store);
      expect(host.textContent).toContain(UNKNOWN_NOTE);
      expectNoReceipt();
      expect(button(NEW)?.disabled ?? true).toBe(true);
      expect(button(PRINT)?.disabled ?? true).toBe(true);
      button(RETRY)!.click();
      expect(store.submit).toHaveBeenCalledTimes(1);
    });

    it("H6 New after a save calls startNew", async () => {
      const store = stubStore(PATHS[which], { outcome: "saved" });
      await mount(store);
      button(NEW)!.click();
      expect(store.startNew).toHaveBeenCalledTimes(1);
    });

    it("H7 the result follows the outcome as it changes: saving, then saved", async () => {
      const store = stubStore(PATHS[which], { outcome: "saving" });
      await mount(store);
      expectNoReceipt();
      store._setOutcome("saved");
      expect(host.textContent).toContain(DOC);
      expect(button(NEW)?.disabled).toBe(false);
    });

    it("H8 English follows the language", async () => {
      setLang("en");
      const store = stubStore(PATHS[which], { outcome: "saved" });
      await mount(store);
      expect(host.textContent).not.toContain(HEADING);
      expect(host.textContent).toContain(DOC);
    });
  });
}
