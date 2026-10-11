// #543 Part 1, DK-G: step 2, booking (design docs/design/xchange-v3-wave3-desk.md §3.2 row "2 Booking", §3.6b row
// "Step 2 booking", §3.9 case 5, D-10, §5.3 step-2 names, §6 row BookingStep). Every test runs on both OWNER_DESK and
// STAFF_DESK (§10). Written before the implementation by the test-writer (claude/709ec749, on the FE lane controller
// claude/5edf7bb5's order). Do not edit, delete or weaken these tests; add your own elsewhere.
//
// [required] by these tests (each module is reached through import.meta.glob, so a missing file fails an assertion):
// - `steps/booking/step.tsx` exports `step: DeskStep` with `index: 2` and `component({ store })`.
// - `steps/booking/i18n.ts` exports `th` and `en` with the same keys; Thai is v2's copy verbatim (§3.6b "Step 2").
// - Controls, by v2's names: `input[name=name]` (read-only, the passport name), `input[name=staff]` (read-only, the
//   signed-in username, §3.9 case 5), `input[name=valueDate]`; the form submits with its `button[type=submit]`; the
//   back button is v2's `← แก้ไขข้อมูลลูกค้า`.
// - No client-made booking reference (D-10): the server numbers the exchange. The step never calls `setBooking` and no
//   `BK-` text appears.
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
  expect(l, `web/src/screens/desk/steps/booking/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

const HEADING = "ข้อมูลการจอง";
const BACK = "← แก้ไขข้อมูลลูกค้า";
const SUBMIT = "ยืนยันการจอง →";
const NEED_NAME_STAFF = "กรุณาระบุชื่อลูกค้าและรหัสพนักงาน";

const PATHS = { OWNER_DESK: { kind: "owner" }, STAFF_DESK: { kind: "staff" } } as const;

function stubStore(path: unknown, init: { name?: string; staff?: string; valueDate?: string } = {}) {
  const [fields] = createSignal({
    name: init.name ?? "JOHN DOE",
    passportNo: "AA1234567",
    nationality: "USA",
    birthDate: "",
    passportExpiry: "",
  });
  const [valueDate, setValueDate] = createSignal(init.valueDate ?? "2026-10-11");
  const [booking, setBooking] = createSignal("");
  const [step] = createSignal(2);
  return {
    path,
    step,
    fields,
    staff: () => init.staff ?? "ploy",
    valueDate,
    booking,
    entry: () => undefined,
    setValueDate: vi.fn((v: string) => setValueDate(v)),
    setBooking: vi.fn((v: string) => setBooking(v)),
    go: vi.fn(),
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => {
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
  vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new Error("step 2 must not call the network"))));
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

const control = (name: string) => host.querySelector<HTMLInputElement>(`[name="${name}"]`);
const button = (text: string) =>
  [...host.querySelectorAll<HTMLButtonElement>("button")].find((b) => (b.textContent ?? "").trim() === text);
const submit = () => host.querySelector<HTMLButtonElement>('button[type="submit"]');

describe("steps/booking: the module", () => {
  it("G0 exports step 2 with a label from its own i18n table", async () => {
    const { step } = await load(stepMods, "./step.tsx");
    expect(step.index).toBe(2);
    expect(step.label().trim()).not.toBe("");
  });

  it("G0b i18n.ts has th and en with the same keys; th carries v2's step-2 copy verbatim", async () => {
    const { th, en } = await load(textMods, "./i18n.ts");
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    expect((th as Record<string, unknown>).zh).toBeUndefined();
    for (const s of [HEADING, BACK, SUBMIT, NEED_NAME_STAFF]) expect(Object.values(th), s).toContain(s);
  });

  it("G0c no money arithmetic, no fetch( and no client-made BK- reference in the step's sources (W7, D-10)", () => {
    expect(Object.keys(SOURCES)).toContain("./step.tsx");
    for (const [file, src] of Object.entries(SOURCES)) {
      expect(src, file).not.toMatch(/Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100/);
      expect(src, file).not.toMatch(/\bfetch\(/);
      expect(src, file).not.toMatch(/BK-|randomUUID/);
    }
  });
});

for (const which of ["OWNER_DESK", "STAFF_DESK"] as const) {
  describe(`steps/booking on ${which}`, () => {
    it("G1 shows v2's heading, the passport name and the signed-in staff, both read-only", async () => {
      const store = stubStore(PATHS[which], { name: "JOHN DOE", staff: "ploy" });
      await mount(store);
      expect(host.textContent).toContain(HEADING);
      expect(control("name")?.value).toBe("JOHN DOE");
      expect(control("name")?.readOnly).toBe(true);
      expect(control("staff")?.value).toBe("ploy");
      expect(control("staff")?.readOnly, "staff is the session's, never typed (§3.9 case 5)").toBe(true);
    });

    it("G2 the value date is the store's and an edit goes to setValueDate as typed", async () => {
      const store = stubStore(PATHS[which]);
      await mount(store);
      const vd = control("valueDate")!;
      expect(vd.value).toBe("2026-10-11");
      vd.value = "2026-10-12";
      vd.dispatchEvent(new Event("input", { bubbles: true }));
      vd.dispatchEvent(new Event("change", { bubbles: true }));
      expect(store.setValueDate).toHaveBeenCalledWith("2026-10-12");
    });

    it("G3 submit goes to step 3 and makes no booking reference (D-10)", async () => {
      const store = stubStore(PATHS[which]);
      await mount(store);
      expect(submit()?.textContent?.trim()).toBe(SUBMIT);
      submit()!.click();
      expect(store.go).toHaveBeenCalledWith(3);
      expect(store.setBooking).not.toHaveBeenCalled();
      expect(host.textContent).not.toMatch(/BK-/);
    });

    it("G4 with no customer name, or no staff code, submit is refused with v2's sentence and stays", async () => {
      for (const init of [{ name: "" }, { staff: "" }]) {
        const store = stubStore(PATHS[which], init);
        await mount(store);
        submit()!.click();
        expect(host.textContent, JSON.stringify(init)).toContain(NEED_NAME_STAFF);
        expect(store.go, JSON.stringify(init)).not.toHaveBeenCalled();
        dispose?.();
        host.innerHTML = "";
      }
    });

    it("G5 back goes to step 1", async () => {
      const store = stubStore(PATHS[which]);
      await mount(store);
      button(BACK)!.click();
      expect(store.go).toHaveBeenCalledWith(1);
    });

    it("G6 English follows the language", async () => {
      setLang("en");
      const store = stubStore(PATHS[which]);
      await mount(store);
      const { en } = await load(textMods, "./i18n.ts");
      expect(host.textContent).not.toContain(HEADING);
      expect(Object.values(en).some((v) => (host.textContent ?? "").includes(v))).toBe(true);
    });
  });
}
