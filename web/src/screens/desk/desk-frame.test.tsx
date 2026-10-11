// V3-desk DK-C0: the desk page frame (main.tsx, DeskPage, steps.ts, the stub store, i18n, css).
// Design: docs/design/xchange-v3-wave3-desk.md §5.1 (layout, the step glob), §5.3 (DeskStep), §6.
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or
// weaken these tests; add your own in another file.
//
// The frame names no quote, rate, payment, customer or document field and makes no network call
// (lane controller's conditions on #539). Files are reached through import.meta.glob, so a missing
// file fails an assertion here instead of the whole file.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { setLang } from "../../shared/i18n";

type StepsModule = typeof import("./steps");
type StoreModule = typeof import("./frame-store");
type PageModule = typeof import("./DeskPage");
type TextsModule = typeof import("./i18n");

const stepsMods = import.meta.glob<StepsModule>("./steps.ts");
const storeMods = import.meta.glob<StoreModule>("./frame-store.ts");
const pageMods = import.meta.glob<PageModule>("./DeskPage.tsx");
const textMods = import.meta.glob<TextsModule>("./i18n.ts");
const FRAME_SOURCES = import.meta.glob<string>(
  [
    "./main.tsx",
    "./DeskPage.tsx",
    "./steps.ts",
    "./frame-store.ts",
    "./i18n.ts",
  ],
  { query: "?raw", import: "default", eager: true },
);

async function load<T>(
  mods: Record<string, () => Promise<T>>,
  key: string,
): Promise<T> {
  const l = mods[key];
  expect(l, `web/src/screens/desk/${key.slice(2)} exists`).toBeDefined();
  return l!();
}
const stepsModule = () => load(stepsMods, "./steps.ts");
const storeModule = () => load(storeMods, "./frame-store.ts");
const pageModule = () => load(pageMods, "./DeskPage.tsx");
const textsModule = () => load(textMods, "./i18n.ts");

type Idx = 0 | 1 | 2 | 3 | 4;

/** A fake step module as the glob would hand it over. */
function stepMod(
  index: Idx,
  label: string,
  body?: (p: { store: { go(i: Idx): void } }) => JSX.Element,
) {
  return {
    step: {
      index,
      label: () => label,
      component:
        body ??
        ((p: { store: { go(i: Idx): void } }) => (
          <div data-testid={`body-${index}`}>
            <button
              type="button"
              data-testid={`next-${index}`}
              onClick={() => p.store.go((index + 1) as Idx)}
            >
              next
            </button>
            <button
              type="button"
              data-testid={`back-${index}`}
              onClick={() => p.store.go((index - 1) as Idx)}
            >
              back
            </button>
          </div>
        )),
    },
  };
}

const FIVE = {
  "./steps/rate/step.tsx": stepMod(0, "Rate"),
  "./steps/customer/step.tsx": stepMod(1, "Passport"),
  "./steps/booking/step.tsx": stepMod(2, "Booking"),
  "./steps/payment/step.tsx": stepMod(3, "Payment"),
  "./steps/receipt/step.tsx": stepMod(4, "Receipt"),
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let fetchSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  setLang("th");
  host = document.createElement("div");
  document.body.appendChild(host);
  fetchSpy = vi.fn(() =>
    Promise.reject(new Error("the desk frame must not call the network")),
  );
  vi.stubGlobal("fetch", fetchSpy);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
  setLang("th");
});

describe("DESK_FLOW: the Blueprint desk flow, cited not re-derived", () => {
  it("F1 has the five step indexes 0..4 in order", async () => {
    const { DESK_FLOW } = await stepsModule();
    expect(DESK_FLOW.map((s) => s.index)).toEqual([0, 1, 2, 3, 4]);
  });

  it('F2 follows Blueprint §03 B03 "quote → ตรวจ → ยืนยันรายการ": step 0 quote, steps 1–3 check, step 4 confirm', async () => {
    const { DESK_FLOW } = await stepsModule();
    expect(DESK_FLOW.map((s) => s.phase)).toEqual([
      "quote",
      "check",
      "check",
      "check",
      "confirm",
    ]);
  });

  it("F3 the steps.ts source cites the Blueprint B03 flow by its section and row", () => {
    const src = FRAME_SOURCES["./steps.ts"];
    expect(src, "steps.ts exists").toBeDefined();
    expect(src).toMatch(/§03/);
    expect(src).toMatch(/B03/);
    expect(src).toContain("quote → ตรวจ → ยืนยันรายการ");
  });
});

describe("collectSteps: the steps come from ./steps/<name>/step.tsx", () => {
  it("C1 returns the steps sorted by index, whatever order the glob gives", async () => {
    const { collectSteps } = await stepsModule();
    const shuffled = {
      "./steps/receipt/step.tsx": FIVE["./steps/receipt/step.tsx"],
      "./steps/rate/step.tsx": FIVE["./steps/rate/step.tsx"],
      "./steps/payment/step.tsx": FIVE["./steps/payment/step.tsx"],
    };
    expect(collectSteps(shuffled).map((s) => s.index)).toEqual([0, 3, 4]);
  });

  it("C2 an empty glob gives no steps (every step ticket is still to come)", async () => {
    const { collectSteps } = await stepsModule();
    expect(collectSteps({})).toEqual([]);
  });

  it("C3 two steps with one index throw, naming both files", async () => {
    const { collectSteps } = await stepsModule();
    const glob = {
      "./steps/rate/step.tsx": stepMod(0, "A"),
      "./steps/other/step.tsx": stepMod(0, "B"),
    };
    expect(() => collectSteps(glob)).toThrow(
      /steps\/rate\/step\.tsx[\s\S]*steps\/other\/step\.tsx|steps\/other\/step\.tsx[\s\S]*steps\/rate\/step\.tsx/,
    );
  });

  it("C4 an index outside DESK_FLOW throws", async () => {
    const { collectSteps } = await stepsModule();
    const bad = {
      "./steps/extra/step.tsx": {
        step: { index: 5, label: () => "X", component: () => null },
      },
    };
    expect(() => collectSteps(bad)).toThrow(/index/i);
  });

  it("C5 a module without a `step` export throws, naming the file", async () => {
    const { collectSteps } = await stepsModule();
    expect(() => collectSteps({ "./steps/rate/step.tsx": {} })).toThrow(
      /steps\/rate\/step\.tsx/,
    );
  });

  it("C6 a path that is not ./steps/<lower-case-name>/step.tsx throws", async () => {
    const { collectSteps } = await stepsModule();
    expect(() =>
      collectSteps({ "./steps/Rate/step.tsx": stepMod(0, "A") }),
    ).toThrow();
    expect(() =>
      collectSteps({ "./steps/rate/index.tsx": stepMod(0, "A") }),
    ).toThrow();
  });
});

describe("navigation: back to any earlier present step, forward only to the next present one", () => {
  it("N1 nextIndex and prevIndex skip absent steps", async () => {
    const { nextIndex, prevIndex } = await stepsModule();
    const present: Idx[] = [0, 2, 4];
    expect(nextIndex(0, present)).toBe(2);
    expect(nextIndex(2, present)).toBe(4);
    expect(nextIndex(4, present)).toBeUndefined();
    expect(prevIndex(4, present)).toBe(2);
    expect(prevIndex(0, present)).toBeUndefined();
  });

  it("N2 canGo: back to any earlier present step; forward only to the next present step; never to an absent one", async () => {
    const { canGo } = await stepsModule();
    const all: Idx[] = [0, 1, 2, 3, 4];
    expect(canGo(3, 0, all)).toBe(true);
    expect(canGo(3, 2, all)).toBe(true);
    expect(canGo(1, 2, all)).toBe(true);
    expect(canGo(1, 3, all)).toBe(false);
    expect(canGo(0, 4, all)).toBe(false);
    expect(canGo(2, 2, all)).toBe(false);
    expect(canGo(0, 2, [0, 2])).toBe(true);
    expect(canGo(0, 1, [0, 2])).toBe(false);
  });

  it("N3 the stub store starts at the lowest present step, and go() obeys canGo", async () => {
    const { createFrameStore } = await storeModule();
    const s = createFrameStore([1, 2, 3]);
    expect(s.step()).toBe(1);
    s.go(3);
    expect(s.step()).toBe(1);
    s.go(2);
    expect(s.step()).toBe(2);
    s.go(3);
    s.go(1);
    expect(s.step()).toBe(1);
    s.go(0);
    expect(s.step()).toBe(1);
  });

  it("N4 the stub store with no present step sits at 0 and ignores go()", async () => {
    const { createFrameStore } = await storeModule();
    const s = createFrameStore([]);
    expect(s.step()).toBe(0);
    s.go(1);
    expect(s.step()).toBe(0);
  });

  it("N5 the stub store says, in its own file, that #551 and DK-C replace it", () => {
    const src = FRAME_SOURCES["./frame-store.ts"];
    expect(src, "frame-store.ts exists").toBeDefined();
    expect(src).toMatch(/#551/);
    expect(src).toMatch(/DK-C\b/);
  });
});

describe("DeskPage: the step bar and the current step", () => {
  async function mount(glob: Record<string, unknown>) {
    const { DeskPage } = await pageModule();
    const { collectSteps } = await stepsModule();
    const { createFrameStore } = await storeModule();
    const steps = collectSteps(glob);
    const store = createFrameStore(steps.map((s) => s.index));
    dispose = render(() => <DeskPage store={store} steps={steps} />, host);
    return store;
  }

  it('P1 the bar is v2\'s ordered list (desk.html: <ol class="steps" aria-label="ขั้นตอนทำรายการ">), one item per present step, "01" … with the step\'s label', async () => {
    await mount(FIVE);
    const ol = host.querySelector("ol.steps");
    expect(ol).not.toBeNull();
    expect(ol!.getAttribute("aria-label")).toBe("ขั้นตอนทำรายการ");
    const items = [...ol!.querySelectorAll("li")].map((li) =>
      li.textContent!.replace(/\s+/g, " ").trim(),
    );
    expect(items).toEqual([
      "01 Rate",
      "02 Passport",
      "03 Booking",
      "04 Payment",
      "05 Receipt",
    ]);
  });

  it('P2 the number comes from the index, not the position: steps 0 and 3 only show "01" and "04"', async () => {
    await mount({
      "./steps/rate/step.tsx": stepMod(0, "Rate"),
      "./steps/payment/step.tsx": stepMod(3, "Payment"),
    });
    const items = [...host.querySelectorAll("ol.steps li")].map((li) =>
      li.textContent!.replace(/\s+/g, " ").trim(),
    );
    expect(items).toEqual(["01 Rate", "04 Payment"]);
  });

  it('P3 only the current step\'s body is rendered, and its item carries aria-current="step"', async () => {
    await mount(FIVE);
    expect(host.querySelector('[data-testid="body-0"]')).not.toBeNull();
    expect(host.querySelector('[data-testid="body-1"]')).toBeNull();
    const current = host.querySelectorAll('ol.steps li[aria-current="step"]');
    expect(current.length).toBe(1);
    expect(current[0]!.textContent).toMatch(/Rate/);
  });

  it("P4 a step moves the page through store.go: forward to the next, back to the previous", async () => {
    await mount(FIVE);
    (host.querySelector('[data-testid="next-0"]') as HTMLButtonElement).click();
    expect(host.querySelector('[data-testid="body-1"]')).not.toBeNull();
    expect(
      host.querySelector('ol.steps li[aria-current="step"]')!.textContent,
    ).toMatch(/Passport/);
    (host.querySelector('[data-testid="back-1"]') as HTMLButtonElement).click();
    expect(host.querySelector('[data-testid="body-0"]')).not.toBeNull();
  });

  it("P5 the bar is not a navigation: clicking an item does not change the step (v2 items are plain <li>)", async () => {
    await mount(FIVE);
    (host.querySelectorAll("ol.steps li")[3] as HTMLLIElement).click();
    expect(host.querySelector('[data-testid="body-0"]')).not.toBeNull();
    expect(host.querySelectorAll("ol.steps button, ol.steps a").length).toBe(0);
  });

  it("P6 with no step yet the page shows the empty text and no bar items", async () => {
    await mount({});
    expect(host.querySelectorAll("ol.steps li").length).toBe(0);
    const { th } = await textsModule();
    expect(host.textContent).toContain(th.empty);
  });

  it("P7 rendering and moving through all five steps makes no network call", async () => {
    await mount(FIVE);
    for (const i of [0, 1, 2, 3] as Idx[]) {
      (
        host.querySelector(`[data-testid="next-${i}"]`) as HTMLButtonElement
      ).click();
    }
    expect(host.querySelector('[data-testid="body-4"]')).not.toBeNull();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('P8 the bar\'s label follows the language: Thai by default, English after setLang("en")', async () => {
    await mount(FIVE);
    const { en } = await textsModule();
    setLang("en");
    expect(host.querySelector("ol.steps")!.getAttribute("aria-label")).toBe(
      en.stepsLabel,
    );
  });

  it("P9 a step whose store is the caller's own type still renders (DeskStep is generic over the store)", async () => {
    const { DeskPage } = await pageModule();
    const [step, setStep] = createSignal<Idx>(0);
    const custom = { step, go: (i: Idx) => setStep(i), extra: "kept" };
    const steps = [
      {
        index: 0 as Idx,
        label: () => "Only",
        component: (p: { store: typeof custom }) => (
          <span data-testid="extra">{p.store.extra}</span>
        ),
      },
    ];
    dispose = render(() => <DeskPage store={custom} steps={steps} />, host);
    expect(host.querySelector('[data-testid="extra"]')!.textContent).toBe(
      "kept",
    );
  });
});

describe("the frame's own text: TH/EN only (a staff screen, #576)", () => {
  it("T1 th and en have the same keys, no empty text, and th is Thai", async () => {
    const { th, en } = await textsModule();
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [k, v] of Object.entries(th)) {
      expect(v, k).not.toBe("");
      expect(v, k).toMatch(/[฀-๿]/);
    }
    for (const [k, v] of Object.entries(en)) expect(v, k).not.toBe("");
  });

  it('T2 th.stepsLabel is v2\'s "ขั้นตอนทำรายการ" verbatim', async () => {
    const { th } = await textsModule();
    expect(th.stepsLabel).toBe("ขั้นตอนทำรายการ");
  });

  it("T3 the table exports no zh: the desk is a staff screen", async () => {
    const mod = (await textsModule()) as Record<string, unknown>;
    expect(Object.keys(mod).sort()).toEqual(["en", "th"]);
  });
});

describe("the frame names no domain field and calls no network (controller conditions 1 and 4, #539)", () => {
  const FILES = [
    "./main.tsx",
    "./DeskPage.tsx",
    "./steps.ts",
    "./frame-store.ts",
  ];

  function code(src: string): string {
    // comments may cite the Blueprint; only code is checked
    return src
      .replace(/\/\*[\s\S]*?\*\//g, "")
      .replace(/(^|[^:])\/\/.*$/gm, "$1");
  }

  it.each(FILES)("S1 %s exists", (f) => {
    expect(
      FRAME_SOURCES[f],
      `web/src/screens/desk/${f.slice(2)} exists`,
    ).toBeDefined();
  });

  it.each(FILES)("S2 %s reads no store member other than step and go", (f) => {
    const src = code(FRAME_SOURCES[f] ?? "");
    const members = [...src.matchAll(/\bstore\.(\w+)/g)].map((m) => m[1]);
    expect(members.filter((m) => m !== "step" && m !== "go")).toEqual([]);
  });

  it.each(FILES)(
    "S3 %s makes no network call and imports no API client",
    (f) => {
      const src = code(FRAME_SOURCES[f] ?? "");
      expect(src).not.toMatch(/\bfetch\s*\(/);
      expect(src).not.toMatch(/ApiClient|shared\/api|['"`]\/api\//);
    },
  );

  it.each(FILES)(
    "S4 %s names no quote, rate, payment, customer, passport, document, booking or receipt field",
    (f) => {
      const src = code(FRAME_SOURCES[f] ?? "")
        // the B03 phase names of DESK_FLOW are allowed as string literals
        .replace(/['"](quote|check|confirm)['"]/g, '""')
        // the DOM's global `document` (document.getElementById …) is not a document field
        .replace(/\bdocument\s*\./g, "");
      const FORBIDDEN = [
        "quote",
        "rate",
        "payment",
        "customer",
        "passport",
        "document",
        "booking",
        "receipt",
        "amount",
        "currency",
        "fee",
      ];
      // every identifier and string word, split at camelCase and _ boundaries
      const words = [...src.matchAll(/[A-Za-z][A-Za-z0-9_]*/g)].flatMap((m) =>
        m[0]
          .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
          .split(/[\s_]+/)
          .map((w) => w.toLowerCase()),
      );
      expect(words.filter((w) => FORBIDDEN.includes(w))).toEqual([]);
    },
  );

  it("S5 main.tsx boots the frame: it collects the steps and renders DeskPage on the desk store", () => {
    const src = code(FRAME_SOURCES["./main.tsx"] ?? "");
    expect(src).toMatch(/collectSteps\s*\(/);
    expect(src).toMatch(
      /import\.meta\.glob\(\s*['"]\.\/steps\/\*\/step\.tsx['"]/,
    );
    expect(src).toMatch(/createDeskStore\s*\(/);
    expect(src).toMatch(/<DeskPage\b/);
  });
});
