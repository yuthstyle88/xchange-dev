// LN-1 (#576): TH/EN/ZH for customer screens, TH/EN for staff screens.
// Design: docs/design/blueprint/languages.md §2 L1–L9, §3.1–§3.3, §6.1 (as amended by PR 666: a customer page's entry
// calls setAudience('customer'); staff pages keep today's single TH/EN toggle; LN-1 touches no AppShell or screen).
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken these tests;
// add your own in another file. The existing i18n tests must stay green unchanged: staff behaviour does not change.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

const STORAGE_KEY = "xc.lang";

type Core = typeof import("./index");
type Langs = typeof import("./languages");

/** A page that has just loaded the module, reading storage afresh. */
async function fresh(): Promise<{
  core: Core;
  Switch: typeof import("./LangSwitch").LangSwitch;
}> {
  vi.resetModules();
  const core = await import("./index");
  const { LangSwitch } = await import("./LangSwitch");
  return { core, Switch: LangSwitch };
}

const languageMods = import.meta.glob<Langs>("./languages.ts");
async function languages(): Promise<Langs> {
  const l = languageMods["./languages.ts"];
  expect(l, "web/src/shared/i18n/languages.ts exists").toBeDefined();
  return l!();
}

const zhMods = import.meta.glob<{ zh: Record<string, string> }>("./zh.ts", {
  eager: true,
});
function sharedZh(): Record<string, string> {
  const m = zhMods["./zh.ts"];
  expect(m, "web/src/shared/i18n/zh.ts exists").toBeDefined();
  return m!.zh;
}

function placeholders(s: string): string[] {
  return [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort();
}

const CUSTOMER_OK = {
  "/src/screens/customer/rates/i18n.ts": {
    th: { title: "อัตราแลกเปลี่ยน", updated: "อัปเดต {when}" },
    en: { title: "Exchange rates", updated: "Updated {when}" },
    zh: { title: "汇率", updated: "更新于 {when}" },
  },
};
const STAFF_OK = {
  "/src/screens/devices/i18n.ts": {
    th: { title: "อุปกรณ์" },
    en: { title: "Devices" },
  },
};

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  window.localStorage.clear();
  host = document.createElement("div");
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  window.localStorage.clear();
});

describe("languages.ts: the one table of languages and audiences", () => {
  it("A1 LANGS, SPOKEN and CUSTOMER_ROOT", async () => {
    const l = await languages();
    expect([...l.LANGS]).toEqual(["th", "en", "zh"]);
    expect([...l.SPOKEN.customer]).toEqual(["th", "en", "zh"]);
    expect([...l.SPOKEN.staff]).toEqual(["th", "en"]);
    expect(l.CUSTOMER_ROOT).toBe("customer");
  });

  it("A2 LOCALE, HTML_LANG and NATIVE_NAME for each language", async () => {
    const l = await languages();
    expect(l.LOCALE).toEqual({ th: "th-TH", en: "en-GB", zh: "zh-CN" });
    expect(l.HTML_LANG).toEqual({ th: "th", en: "en", zh: "zh-CN" });
    expect(l.NATIVE_NAME).toEqual({ th: "ไทย", en: "English", zh: "中文" });
  });
});

describe('the audience starts as staff (TH/EN); only setAudience("customer") opens Chinese', () => {
  it('B1 with no setAudience call, setLang("zh") changes nothing', async () => {
    const { core } = await fresh();
    core.setLang("zh");
    expect(core.lang()).toBe("th");
    expect(window.localStorage.getItem(STORAGE_KEY)).not.toBe("zh");
  });

  it('B2 a stored "zh" on a staff page shows Thai, and the stored choice is kept', async () => {
    window.localStorage.setItem(STORAGE_KEY, "zh");
    const { core } = await fresh();
    expect(core.lang()).toBe("th");
    expect(core.locale()).toBe("th-TH");
    expect(document.documentElement.lang).toBe("th");
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe("zh");
  });

  it("B3 staff pages keep TH/EN switching exactly as today", async () => {
    const { core } = await fresh();
    core.setLang("en");
    expect(core.lang()).toBe("en");
    expect(core.locale()).toBe("en-GB");
    core.setLang("th");
    expect(core.lang()).toBe("th");
  });

  it("C1 on a customer page Chinese works: lang, locale, <html lang> and storage", async () => {
    const { core } = await fresh();
    core.setAudience("customer");
    core.setLang("zh");
    expect(core.lang()).toBe("zh");
    expect(core.locale()).toBe("zh-CN");
    expect(document.documentElement.lang).toBe("zh-CN");
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe("zh");
  });

  it('C2 a stored "zh" is used once the page declares itself a customer page', async () => {
    window.localStorage.setItem(STORAGE_KEY, "zh");
    const { core } = await fresh();
    core.setAudience("customer");
    expect(core.lang()).toBe("zh");
  });

  it("C3 the customer choice survives a staff page: Thai there, Chinese again on the next customer page", async () => {
    const first = await fresh();
    first.core.setAudience("customer");
    first.core.setLang("zh");
    const staff = await fresh();
    expect(staff.core.lang()).toBe("th");
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe("zh");
    const again = await fresh();
    again.core.setAudience("customer");
    expect(again.core.lang()).toBe("zh");
  });
});

describe("collectScreens: customer tables carry a complete zh, staff tables never do (§3.2)", () => {
  it("D1 a complete customer table is collected, namespaced like th and en", async () => {
    const { core } = await fresh();
    const s = core.collectScreens({ ...CUSTOMER_OK, ...STAFF_OK });
    expect(s.zh["customer.rates.title"]).toBe("汇率");
    expect(s.th["devices.title"]).toBe("อุปกรณ์");
    expect(Object.keys(s.zh).some((k) => k.startsWith("devices."))).toBe(false);
  });

  it("D2 a customer table without zh throws, naming the screen", async () => {
    const { core } = await fresh();
    const glob = {
      "/src/screens/customer/rates/i18n.ts": {
        th: { title: "ก" },
        en: { title: "A" },
      },
    };
    expect(() => core.collectScreens(glob)).toThrow(/customer\.rates[\s\S]*zh/);
  });

  it.each([
    ["a missing key", { title: "汇率" }, /updated/],
    [
      "an extra key",
      { title: "汇率", updated: "更新于 {when}", extra: "多" },
      /extra/,
    ],
    [
      "a renamed placeholder",
      { title: "汇率", updated: "更新于 {time}" },
      /updated/,
    ],
    ["an empty value", { title: "", updated: "更新于 {when}" }, /title/],
  ])(
    "D3 a customer zh with %s throws, naming the screen and key",
    async (_name, zh, key) => {
      const { core } = await fresh();
      const base = CUSTOMER_OK["/src/screens/customer/rates/i18n.ts"];
      const glob = {
        "/src/screens/customer/rates/i18n.ts": { th: base.th, en: base.en, zh },
      };
      expect(() => core.collectScreens(glob)).toThrow(/customer\.rates/);
      expect(() => core.collectScreens(glob)).toThrow(key);
    },
  );

  it("D4 a staff table that exports zh throws, naming the screen", async () => {
    const { core } = await fresh();
    const glob = {
      "/src/screens/devices/i18n.ts": {
        th: { title: "ก" },
        en: { title: "A" },
        zh: { title: "设备" },
      },
    };
    expect(() => core.collectScreens(glob)).toThrow(
      /devices[\s\S]*(staff|TH\/EN)/,
    );
  });

  it("D5 only the first folder decides: admin/customer is a staff screen", async () => {
    const { core } = await fresh();
    const staffNamedCustomer = {
      "/src/screens/admin/customer/i18n.ts": {
        th: { title: "ลูกค้า" },
        en: { title: "Customer" },
      },
    };
    expect(() => core.collectScreens(staffNamedCustomer)).not.toThrow();
    const withZh = {
      "/src/screens/admin/customer/i18n.ts": {
        th: { title: "ลูกค้า" },
        en: { title: "Customer" },
        zh: { title: "客户" },
      },
    };
    expect(() => core.collectScreens(withZh)).toThrow(/admin\.customer/);
  });
});

describe("lookup on customer pages: zh → en → key, never th and never empty (L6)", () => {
  it("E1 a screen key resolves to its zh text", async () => {
    const { core } = await fresh();
    core.setAudience("customer");
    core.setLang("zh");
    const tr = core.translator(core.collectScreens(CUSTOMER_OK));
    expect(tr("customer.rates.updated", { when: "10:00" })).toBe(
      "更新于 10:00",
    );
  });

  it("E2 a key missing from zh falls back to en, not th", async () => {
    const { core } = await fresh();
    core.setAudience("customer");
    core.setLang("zh");
    const tr = core.translator({
      th: { "x.k": "ไทย" },
      en: { "x.k": "English" },
      zh: {},
    });
    expect(tr("x.k")).toBe("English");
    expect(tr("x.unknown")).toBe("x.unknown");
  });

  it("E3 a shared key reads the shared zh table", async () => {
    const { core } = await fresh();
    core.setAudience("customer");
    core.setLang("zh");
    expect(core.t("shell.langSwitch", { language: "中文" })).toBe(
      sharedZh()["shell.langSwitch"]!.replace("{language}", "中文"),
    );
  });

  it("E4 errorText in Chinese: the table sentence for a known code, else the server message", async () => {
    const { core } = await fresh();
    core.setAudience("customer");
    core.setLang("zh");
    expect(core.errorText("signIn", "server words")).toBe(
      sharedZh()["error.signIn"],
    );
    expect(core.errorText("noSuchCode", "server words")).toBe("server words");
  });
});

describe("the shared zh table is complete (L9)", () => {
  it("F1 zh has every key of th, no empty value and the same placeholders", async () => {
    const { th } = await import("./th");
    const zh = sharedZh();
    expect(Object.keys(zh).sort()).toEqual(Object.keys(th).sort());
    for (const [k, v] of Object.entries(th)) {
      expect(zh[k], k).toBeTruthy();
      expect(placeholders(zh[k]!), k).toEqual(placeholders(v));
    }
  });

  it("F2 zh is not a copy of en: at least the shell and error sentences are translated", async () => {
    const { en } = await import("./en");
    const zh = sharedZh();
    expect(zh["shell.langSwitch"]).not.toBe(
      en["shell.langSwitch" as keyof typeof en],
    );
    expect(zh["error.signIn"]).not.toBe(en["error.signIn" as keyof typeof en]);
  });
});

describe("LangSwitch (§3.3)", () => {
  it("G1 staff: still the single toggle showing the other language", async () => {
    const { Switch } = await fresh();
    dispose = render(() => <Switch />, host);
    const buttons = host.querySelectorAll("button");
    expect(buttons.length).toBe(1);
    expect(buttons[0]!.textContent?.trim()).toBe("EN");
    buttons[0]!.click();
    expect(host.querySelector("button")!.textContent?.trim()).toBe("TH");
  });

  it("G2 customer: one button per language, the current one pressed, labels through t", async () => {
    const { core, Switch } = await fresh();
    core.setAudience("customer");
    dispose = render(() => <Switch />, host);
    const buttons = [...host.querySelectorAll("button")];
    expect(buttons.map((b) => b.textContent?.trim())).toEqual([
      "TH",
      "EN",
      "中文",
    ]);
    expect(buttons.map((b) => b.getAttribute("aria-pressed"))).toEqual([
      "true",
      "false",
      "false",
    ]);
    expect(buttons[2]!.getAttribute("aria-label")).toBe(
      core.t("shell.langSwitch", { language: "中文" }),
    );
  });

  it("G3 customer: pressing 中文 switches to Chinese and moves aria-pressed", async () => {
    const { core, Switch } = await fresh();
    core.setAudience("customer");
    dispose = render(() => <Switch />, host);
    [...host.querySelectorAll("button")][2]!.click();
    expect(core.lang()).toBe("zh");
    expect(
      [...host.querySelectorAll("button")].map((b) =>
        b.getAttribute("aria-pressed"),
      ),
    ).toEqual(["false", "false", "true"]);
  });
});

// §6.1 (added on claude/9e7ea4c7's review of PR 666): the regression guard PR 607's required AppShell prop gave.
// Every customer entry declares its audience before it renders. Passes vacuously until the first customer screen lands.
const customerEntries = import.meta.glob<string>("../../screens/customer/*/main.tsx", {
  query: "?raw",
  import: "default",
  eager: true,
});

/** Index of the first setAudience('customer') call, or -1. Accepts either quote style. */
function audienceCallIndex(src: string): number {
  const m = /setAudience\(\s*(["'])customer\1\s*\)/.exec(src);
  return m ? m.index : -1;
}
/** Index of the first render( call, or -1. */
function firstRenderIndex(src: string): number {
  const m = /\brender\(/.exec(src);
  return m ? m.index : -1;
}

describe("customer entries declare their audience (§6.1)", () => {
  it("H1 every screens/customer/*/main.tsx calls setAudience('customer') before its first render(", () => {
    const bad: string[] = [];
    for (const [path, src] of Object.entries(customerEntries)) {
      const a = audienceCallIndex(src);
      const r = firstRenderIndex(src);
      if (a < 0 || (r >= 0 && a > r)) bad.push(path);
    }
    expect(bad, "customer entries without setAudience('customer') before render(").toEqual([]);
  });

  it("H2 the guard's own reading: a missing call or a call after render( is caught", () => {
    const ok = `import { setAudience } from "x";\nsetAudience('customer');\nrender(() => null, el);`;
    const missing = `render(() => null, el);`;
    const late = `render(() => null, el);\nsetAudience("customer");`;
    expect(audienceCallIndex(ok) >= 0 && audienceCallIndex(ok) < firstRenderIndex(ok)).toBe(true);
    expect(audienceCallIndex(missing)).toBe(-1);
    expect(audienceCallIndex(late) > firstRenderIndex(late)).toBe(true);
  });
});
