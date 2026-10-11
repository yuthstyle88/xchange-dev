// #539 DK-E: step 1, passport / customer (design docs/design/xchange-v3-wave3-desk.md §3.2 rows 1 and "1 skip",
// §3.6b row "Step 1 passport", §3.9 DK-1 customer error; ticket #539 Part 3 §7–§9 and its CI-4 note on nationality;
// lane controller's ruling on #539: `DeskStore` gains `api`, steps reach the server through `props.store.api`).
// Every test runs on both OWNER_DESK and STAFF_DESK (§10).
// Written before the implementation by the test-writer (claude/709ec749, on the FE lane controller claude/5edf7bb5's
// order). Do not edit, delete or weaken these tests; add your own elsewhere.
//
// [required] by these tests (each module is reached through import.meta.glob, so a missing file fails an assertion):
// - `steps/customer/step.tsx` exports `step: DeskStep` with `index: 1`; `steps/customer/i18n.ts` exports `th`/`en`.
// - Controls: `input[type=file][name=passport]`, `input[name=name]`, `input[name=passportNo]`, `select[name=nationality]`
//   (a picker of ISO 3166 alpha-3 codes, never free text), `input[name=birthDate]`, `input[name=passportExpiry]`,
//   `input[type=checkbox][name=identityChecked]`, the form's `button[type=submit]`, a button reading
//   `ข้ามข้อมูลลูกค้า →` (skip) and one reading `← กลับไปเลือกเรท` (back to step 0).
// - Calls go through `store.api` to `store.path.rpc` as v2's RPC `{ action, payload }`:
//   - staff (`path.usesDraft`): `opsBeginTransaction { type, currency, amount }` with the amount **as a string**, once,
//     before the first `reserveCustomer`, then `setStaffDraftId(<answer id>)` (X5);
//   - `reserveCustomer { name, passportNo, nationality, birthDate, passportExpiry, mode: "LIVE" }` → `{ customerId,
//     existing }` → `setCustomerId`, then `go(2)`;
//   - `entry()?.beforeReserve(store)` is awaited before `reserveCustomer`.
// - A refusal shows the server's `error` verbatim; a failure without one shows DK-1's
//   `สร้างรหัสลูกค้าไม่ได้ เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่`.
//
// The skip's nationality is `XXX`, never v2's `N/A` (owner 2026-10-11, design customer-identity-consent.md §1 skip row):
// CI-4 #651 refuses anything that is not 3 letters.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { setLang } from "../../../../shared/i18n";
import { ApiClient } from "../../../../shared/api/client";

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
  expect(l, `web/src/screens/desk/steps/customer/${key.slice(2)} exists`).toBeDefined();
  return l!();
}

const NO_FILE = "กรุณาแนบไฟล์สแกนหรือรูปพาสปอร์ตก่อน";
const MISSING = "กรุณากรอกชื่อ เลขพาสปอร์ต และสัญชาติ";
const HEIC = "รองรับ JPG, PNG, WebP หรือ PDF กรุณาแปลงไฟล์ HEIC เป็น JPG ก่อน";
const TOO_BIG = "กรุณาใช้ไฟล์ที่มีข้อมูลและขนาดไม่เกิน 15 MB";
const NO_SERVER = "สร้างรหัสลูกค้าไม่ได้ เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่";
const HEADING = "พาสปอร์ต / ข้อมูลลูกค้า";
const SKIP = "ข้ามข้อมูลลูกค้า →";
const BACK = "← กลับไปเลือกเรท";

const PATHS = {
  OWNER_DESK: { kind: "owner", rpc: "/api/rpc", usesDraft: false },
  STAFF_DESK: { kind: "staff", rpc: "/api/staff/rpc", usesDraft: true },
} as const;

const JPEG = new File([new Uint8Array([0xff, 0xd8, 0xff, 1])], "passport.jpg", { type: "image/jpeg" });

type RpcCall = { path: string; action: string; payload: any; at: number };

function rpcServer(answer: (action: string, payload: any) => { status: number; body?: unknown; raw?: string; throws?: boolean }) {
  const calls: RpcCall[] = [];
  let clock = 0;
  const fetchFn = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const body = JSON.parse(String(init?.body ?? "{}"));
    const call = { path: String(input), action: body.action, payload: body.payload, at: clock++ };
    calls.push(call);
    const a = answer(call.action, call.payload);
    if (a.throws) throw new TypeError("Failed to fetch");
    return new Response(a.raw ?? JSON.stringify(a.body ?? {}), {
      status: a.status,
      headers: { "content-type": a.raw ? "text/html" : "application/json" },
    });
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), calls, tick: () => clock++ };
}

const OK = (action: string) =>
  action === "opsBeginTransaction"
    ? { status: 200, body: { id: "draft-9" } }
    : { status: 200, body: { customerId: "CX-00000042", existing: false } };

function stubStore(path: (typeof PATHS)[keyof typeof PATHS], api: ApiClient, opts: { entry?: any; staffDraftId?: string } = {}) {
  const [passport, setPassportS] = createSignal<File | undefined>();
  const [fields, setFields] = createSignal({ name: "", passportNo: "", nationality: "", birthDate: "", passportExpiry: "" });
  const [identityChecked, setIC] = createSignal(false);
  const [customerId, setCID] = createSignal("");
  const [staffDraftId, setSDI] = createSignal(opts.staffDraftId ?? "");
  const [entry] = createSignal<any>(opts.entry);
  const store = {
    api,
    path,
    type: () => "BUY",
    currency: () => "USD",
    amount: () => "100.50",
    step: () => 1,
    config: () => ({ shiftOpen: true }),
    passport,
    setPassport: vi.fn((f: File | undefined) => setPassportS(f)),
    fields,
    setField: vi.fn((k: string, v: string, by: "user" | "ocr") => {
      setFields({ ...fields(), [k]: v });
      if (by === "user") setIC(false);
    }),
    editedByUser: () => new Set<string>(),
    identityChecked,
    setIdentityChecked: vi.fn((v: boolean) => setIC(v)),
    customerId,
    setCustomerId: vi.fn((v: string) => setCID(v)),
    staffDraftId,
    setStaffDraftId: vi.fn((v: string) => setSDI(v)),
    entry,
    go: vi.fn(),
  };
  return store;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => {
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
});
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  setLang("th");
});

async function mount(store: ReturnType<typeof stubStore>) {
  const { step } = await load(stepMods, "./step.tsx");
  dispose = render(() => step.component({ store }), host);
}

const q = <T extends Element>(sel: string) => host.querySelector<T>(sel);
const button = (text: string) => [...host.querySelectorAll("button")].find((b) => (b.textContent ?? "").includes(text));

function attach(file: File) {
  const input = q<HTMLInputElement>('input[type="file"][name="passport"]');
  expect(input, "the passport file input").not.toBeNull();
  Object.defineProperty(input!, "files", { value: [file], configurable: true });
  input!.dispatchEvent(new Event("change", { bubbles: true }));
}

function type(name: string, value: string) {
  const el = q<HTMLInputElement>(`input[name="${name}"]`);
  expect(el, `input ${name}`).not.toBeNull();
  el!.value = value;
  el!.dispatchEvent(new Event("input", { bubbles: true }));
  el!.dispatchEvent(new Event("change", { bubbles: true }));
}

function pickNationality(code: string) {
  const sel = q<HTMLSelectElement>('select[name="nationality"]');
  expect(sel, "the nationality picker").not.toBeNull();
  sel!.value = code;
  sel!.dispatchEvent(new Event("change", { bubbles: true }));
}

function fillAll() {
  attach(JPEG);
  type("name", "JOHN DOE");
  type("passportNo", "AA1234567");
  pickNationality("USA");
  type("birthDate", "1990-01-01");
  type("passportExpiry", "2030-01-01");
}

const submit = () => q<HTMLButtonElement>('button[type="submit"]')!.click();

describe("steps/customer: the module", () => {
  it("C0 exports step 1 with a label", async () => {
    const { step } = await load(stepMods, "./step.tsx");
    expect(step.index).toBe(1);
    expect(step.label().trim()).not.toBe("");
  });

  it("C0b i18n.ts has th and en with the same keys; th carries v2's step-1 copy and DK-1's sentence verbatim", async () => {
    const { th, en } = await load(textMods, "./i18n.ts");
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const s of [HEADING, NO_FILE, MISSING, HEIC, TOO_BIG, NO_SERVER, SKIP, BACK]) expect(Object.values(th), s).toContain(s);
  });

  it("C0c no money arithmetic and no fetch( in the step's sources (W7, every call through ApiClient)", () => {
    expect(Object.keys(SOURCES)).toContain("./step.tsx");
    for (const [file, src] of Object.entries(SOURCES)) {
      expect(src, file).not.toMatch(/Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100/);
      expect(src, file).not.toMatch(/\bfetch\(/);
    }
  });
});

for (const [which, path] of Object.entries(PATHS)) {
  describe(`steps/customer on ${which}`, () => {
    it("C1 no file attached: submit is refused with v2's sentence, and nothing is sent", async () => {
      const s = rpcServer((a) => OK(a));
      const store = stubStore(path, s.api);
      await mount(store);
      type("name", "JOHN DOE");
      type("passportNo", "AA1234567");
      pickNationality("USA");
      submit();
      await vi.waitFor(() => expect(host.textContent).toContain(NO_FILE));
      expect(s.calls).toHaveLength(0);
      expect(store.go).not.toHaveBeenCalled();
    });

    it("C2 a missing name, passport number or nationality is refused, and nothing is sent", async () => {
      const s = rpcServer((a) => OK(a));
      const store = stubStore(path, s.api);
      await mount(store);
      attach(JPEG);
      type("name", "JOHN DOE");
      submit();
      await vi.waitFor(() => expect(host.textContent).toContain(MISSING));
      expect(s.calls).toHaveLength(0);
    });

    it("C3 a HEIC file is refused with v2's sentence and not kept", async () => {
      const store = stubStore(path, rpcServer((a) => OK(a)).api);
      await mount(store);
      attach(new File([new Uint8Array([1, 2])], "p.heic", { type: "image/heic" }));
      await vi.waitFor(() => expect(host.textContent).toContain(HEIC));
      expect(store.setPassport).not.toHaveBeenCalledWith(expect.objectContaining({ name: "p.heic" }));
    });

    it("C4 a file over 15 MB is refused with v2's sentence and not kept", async () => {
      const store = stubStore(path, rpcServer((a) => OK(a)).api);
      await mount(store);
      const big = new File([new Uint8Array(15 * 1024 * 1024 + 1)], "big.jpg", { type: "image/jpeg" });
      attach(big);
      await vi.waitFor(() => expect(host.textContent).toContain(TOO_BIG));
      expect(store.setPassport).not.toHaveBeenCalledWith(big);
    });

    it("C5 a JPEG is kept in the store; typing a field is a user edit (it clears identityChecked)", async () => {
      const store = stubStore(path, rpcServer((a) => OK(a)).api);
      await mount(store);
      attach(JPEG);
      expect(store.setPassport).toHaveBeenCalledWith(JPEG);
      type("name", "JOHN");
      expect(store.setField).toHaveBeenCalledWith("name", "JOHN", "user");
    });

    it("C6 nationality is a picker of ISO alpha-3 codes, never free text", async () => {
      await mount(stubStore(path, rpcServer((a) => OK(a)).api));
      expect(q('input[name="nationality"]'), "no free-text nationality").toBeNull();
      const sel = q<HTMLSelectElement>('select[name="nationality"]')!;
      const codes = [...sel.options].map((o) => o.value).filter((v) => v !== "");
      expect(codes.length).toBeGreaterThan(100);
      for (const c of codes) expect(c, `option ${c}`).toMatch(/^[A-Z]{3}$/);
      for (const c of ["THA", "USA", "JPN", "CHN", "DEU"]) expect(codes).toContain(c);
    });

    it("C7 a complete form reserves the customer through store.api on path.rpc and goes to step 2", async () => {
      const s = rpcServer((a) => OK(a));
      const store = stubStore(path, s.api);
      await mount(store);
      fillAll();
      submit();
      await vi.waitFor(() => expect(store.go).toHaveBeenCalledWith(2));
      const reserve = s.calls.find((c) => c.action === "reserveCustomer")!;
      expect(reserve.path).toBe(path.rpc);
      expect(reserve.payload).toEqual({
        name: "JOHN DOE",
        passportNo: "AA1234567",
        nationality: "USA",
        birthDate: "1990-01-01",
        passportExpiry: "2030-01-01",
        mode: "LIVE",
      });
      expect(reserve.payload.nationality).toMatch(/^[A-Z]{3}$/);
      expect(store.setCustomerId).toHaveBeenCalledWith("CX-00000042");
    });

    it(`C8 ${path.usesDraft ? "staff: opsBeginTransaction first, the amount as a string, then the draft id is kept" : "owner: no opsBeginTransaction"}`, async () => {
      const s = rpcServer((a) => OK(a));
      const store = stubStore(path, s.api);
      await mount(store);
      fillAll();
      submit();
      await vi.waitFor(() => expect(store.go).toHaveBeenCalledWith(2));
      const begin = s.calls.filter((c) => c.action === "opsBeginTransaction");
      if (path.usesDraft) {
        expect(begin).toHaveLength(1);
        expect(begin[0].path).toBe(path.rpc);
        expect(begin[0].payload).toEqual({ type: "BUY", currency: "USD", amount: "100.50" });
        expect(typeof begin[0].payload.amount).toBe("string");
        const reserve = s.calls.find((c) => c.action === "reserveCustomer")!;
        expect(begin[0].at).toBeLessThan(reserve.at);
        expect(store.setStaffDraftId).toHaveBeenCalledWith("draft-9");
      } else {
        expect(begin).toHaveLength(0);
      }
    });

    if (path.usesDraft) {
      it("C8b staff: a draft already begun is not begun again", async () => {
        const s = rpcServer((a) => OK(a));
        const store = stubStore(path, s.api, { staffDraftId: "draft-1" });
        await mount(store);
        fillAll();
        submit();
        await vi.waitFor(() => expect(store.go).toHaveBeenCalledWith(2));
        expect(s.calls.filter((c) => c.action === "opsBeginTransaction")).toHaveLength(0);
      });
    }

    it("C9 an entry's beforeReserve is awaited before reserveCustomer", async () => {
      const order: string[] = [];
      let release!: () => void;
      const held = new Promise<void>((r) => (release = r));
      const s = rpcServer((a) => {
        order.push(a);
        return OK(a);
      });
      const entry = {
        kind: "kiosk",
        id: "k1",
        locked: true,
        banner: () => "",
        identityConfirmed: () => false,
        setIdentityConfirmed: () => undefined,
        payloadExtras: () => ({}),
        refusal: () => undefined,
        beforeReserve: vi.fn(async () => {
          order.push("beforeReserve:start");
          await held;
          order.push("beforeReserve:end");
        }),
      };
      const store = stubStore(path, s.api, { entry });
      await mount(store);
      fillAll();
      submit();
      await vi.waitFor(() => expect(entry.beforeReserve).toHaveBeenCalled());
      expect(order).not.toContain("reserveCustomer");
      release();
      await vi.waitFor(() => expect(order).toContain("reserveCustomer"));
      expect(order.indexOf("beforeReserve:end")).toBeLessThan(order.indexOf("reserveCustomer"));
    });

    it("C10 a refusal shows the server's error verbatim, and the step stays", async () => {
      const s = rpcServer((a) => (a === "reserveCustomer" ? { status: 400, body: { error: "เลขพาสปอร์ตไม่ถูกต้อง" } } : OK(a)));
      const store = stubStore(path, s.api);
      await mount(store);
      fillAll();
      submit();
      await vi.waitFor(() => expect(host.textContent).toContain("เลขพาสปอร์ตไม่ถูกต้อง"));
      expect(store.go).not.toHaveBeenCalledWith(2);
    });

    for (const [what, a] of [
      ["a network failure", { status: 0, throws: true }],
      ["a 5xx that is not JSON", { status: 502, raw: "<html>bad gateway</html>" }],
    ] as const) {
      it(`C11 ${what} shows DK-1's sentence`, async () => {
        const s = rpcServer((act) => (act === "reserveCustomer" ? a : OK(act)));
        const store = stubStore(path, s.api);
        await mount(store);
        fillAll();
        submit();
        await vi.waitFor(() => expect(host.textContent).toContain(NO_SERVER));
        expect(store.go).not.toHaveBeenCalledWith(2);
      });
    }

    it("C12 skip: a PNG stands in for the passport, WALK-IN / WALKIN / XXX, then reserveCustomer and step 2 (X3)", async () => {
      const s = rpcServer((a) => OK(a));
      const store = stubStore(path, s.api);
      await mount(store);
      const skip = button(SKIP);
      expect(skip, "the skip button").toBeDefined();
      skip!.click();
      await vi.waitFor(() => expect(store.go).toHaveBeenCalledWith(2));
      const png = store.setPassport.mock.calls.map((c) => c[0]).find((f) => f instanceof File && f.type === "image/png");
      expect(png, "a PNG passport stand-in").toBeDefined();
      const reserve = s.calls.find((c) => c.action === "reserveCustomer")!;
      expect(reserve.payload.name).toBe("WALK-IN");
      expect(reserve.payload.passportNo).toBe("WALKIN");
      expect(reserve.payload.nationality, "the skip sends XXX (owner 2026-10-11)").toBe("XXX");
      expect(JSON.stringify(s.calls), "v2's N/A is never sent").not.toContain("N/A");
    });

    it("C13 the identity checkbox writes to the store", async () => {
      const store = stubStore(path, rpcServer((a) => OK(a)).api);
      await mount(store);
      const box = q<HTMLInputElement>('input[type="checkbox"][name="identityChecked"]')!;
      expect(box).not.toBeNull();
      box.click();
      expect(store.setIdentityChecked).toHaveBeenCalledWith(true);
    });

    it("C14 the heading is v2's, back goes to step 0, and the step renders without the optional OCR part", async () => {
      const store = stubStore(path, rpcServer((a) => OK(a)).api);
      await mount(store);
      expect(host.textContent).toContain(HEADING);
      button(BACK)!.click();
      expect(store.go).toHaveBeenCalledWith(0);
    });
  });
}
