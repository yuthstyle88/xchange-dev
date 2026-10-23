// #4922 (T22, docs/design/web-input-validation.md §2, §6): the members segment form and the consent
// retention row refuse what the server refuses, before any request, with a worded line.
//
// "At the field" means: an element `[data-field-error="<field>"]` holds the message, where <field> is
// the key passed to `fieldError` as a label (`name`, `rule.recencyDays`, `rule.frequency`,
// `rule.valueThb`). A browser validation bubble is not visible here, so a `required` attribute alone
// does not satisfy MS1: the worded line must be rendered.
//
// | #   | Input                          | Expect                              |
// | MS1 | segment name blank             | worded line at name; no request     |
// | MS2 | recency days 4294967296        | worded line; no request             |
// | MS3 | server 400 field `name`        | line at name                        |
// | CR1 | retention 2147483648           | periodInvalid; no request           |
// | CR2 | retention 30                   | saved as today                      |

import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { setLang } from "../../../shared/i18n";
import { en as sharedEn } from "../../../shared/i18n/en";
import {
  makeConsentTextsData,
  makeRetentionSetOutput,
} from "../../../shared/testing/fixtures";
import { warmModules } from "../../../shared/testing/warm-modules";
import { Consent } from "../consent/Consent";
import * as consentTable from "../consent/i18n";
import type { AdminMenuProps } from "../menus";
import { Toast, toast } from "../parts/toast";
import * as table from "./i18n";
import { Members } from "./Members";

beforeAll(
  () =>
    warmModules(
      () => import("./Members"),
      () => import("./i18n"),
      () => import("../consent/Consent"),
      () => import("../consent/i18n"),
      () => import("../../../shared/api/client"),
      () => import("../../../shared/i18n"),
      () => import("../../../shared/i18n/field-error"),
      () => import("../../../shared/i18n/worded-error"),
    ),
  60_000,
);

interface Captured {
  method: string;
  pathname: string;
  body: Record<string, unknown>;
}

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 10; i++) {
    await new Promise((r) => realSetTimeout(r, 0));
  }
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function makeProps(): AdminMenuProps {
  const [branch] = createSignal("MAIN");
  const [period] = createSignal({ from: "2026-10-16", to: "2026-10-16" });
  const [shown] = createSignal(true);
  return { branch, period, shown };
}

let root: HTMLDivElement;
let dispose: (() => void) | undefined;
let writes: Captured[];
/** What POST /api/segments answers; the default is a 200 define output. */
let defineAnswer: () => Response;

beforeEach(() => {
  setLang("en");
  root = document.createElement("div");
  document.body.appendChild(root);
  writes = [];
  defineAnswer = () =>
    jsonResponse(200, { id: `seg-${writes.length}`, version: 1 });

  vi.stubGlobal(
    "fetch",
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const rawUrl =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const u = new URL(rawUrl, "http://localhost");
      const method = (init?.method ?? "GET").toUpperCase();

      if (u.pathname.includes("views/consent_texts")) {
        return jsonResponse(200, {
          version: 1,
          changed: true,
          data: makeConsentTextsData(),
        });
      }
      if (method !== "GET") {
        const raw = typeof init?.body === "string" ? init.body : "{}";
        writes.push({
          method,
          pathname: u.pathname,
          body: JSON.parse(raw) as Record<string, unknown>,
        });
        if (method === "POST" && u.pathname === "/api/segments")
          return defineAnswer();
        if (
          method === "PUT" &&
          u.pathname.startsWith("/api/retention-policies/")
        ) {
          return jsonResponse(
            200,
            makeRetentionSetOutput({
              dataType: "IDENTITY_VALUE",
              periodDays: 30,
            }),
          );
        }
      }
      return jsonResponse(404, {
        error: { code: "notFound", message: "Not found" },
      });
    },
  );
});

afterEach(async () => {
  await settle();
  dispose?.();
  dispose = undefined;
  root.remove();
  document.body.innerHTML = "";
  toast("");
  setLang("th");
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function setInput(input: HTMLInputElement | null, val: string): void {
  expect(input).not.toBeNull();
  // Keep raw strings such as '4294967296' exactly as typed (no number sanitising).
  input!.type = "text";
  input!.value = val;
  input!.dispatchEvent(new Event("input", { bubbles: true }));
  input!.dispatchEvent(new Event("change", { bubbles: true }));
}

function fieldLine(field: string): string | null {
  const el = root.querySelector(`[data-field-error="${field}"]`);
  const text = el?.textContent?.trim() ?? "";
  return text === "" ? null : text;
}

function bannerText(): string | null {
  const el = root.querySelector('[role="alert"]');
  const text = el?.textContent?.trim() ?? "";
  return text === "" ? null : text;
}

describe("#4922 Members segment define", () => {
  async function mountMembers(): Promise<void> {
    dispose = render(() => <Members {...makeProps()} />, root);
    await settle();
  }

  function define(): void {
    const btn = root.querySelector<HTMLButtonElement>(
      'button[data-action="define"]',
    );
    expect(btn).not.toBeNull();
    btn!.click();
  }

  it("MS1: a blank (spaces only) segment name shows a worded line at name and sends nothing", async () => {
    await mountMembers();
    setInput(root.querySelector<HTMLInputElement>("#segment-name"), "   ");
    setInput(
      root.querySelector<HTMLInputElement>("#segment-recency-days"),
      "30",
    );

    define();
    await settle();

    expect(writes).toHaveLength(0);
    const line = fieldLine("name");
    expect(line, 'a worded line at [data-field-error="name"]').not.toBeNull();
    expect(line).not.toBe("validationFailed");
  });

  it("MS2: recency days 4294967296 (above u32) shows a worded line and sends nothing; 4294967295 is sent", async () => {
    await mountMembers();
    setInput(root.querySelector<HTMLInputElement>("#segment-name"), "VIP");
    setInput(
      root.querySelector<HTMLInputElement>("#segment-recency-days"),
      "4294967296",
    );

    define();
    await settle();

    expect(writes).toHaveLength(0);
    const line = fieldLine("rule.recencyDays") ?? bannerText();
    expect(line, "a worded line at the field or in the banner").not.toBeNull();

    setInput(
      root.querySelector<HTMLInputElement>("#segment-recency-days"),
      "4294967295",
    );
    define();
    await settle();

    expect(writes).toHaveLength(1);
    expect(writes[0]!.body).toEqual({
      name: "VIP",
      rule: { recency_days: 4294967295 },
    });
  });

  it('MS3: a server 400 validationFailed with field "name" shows input.invalid with the name label at name', async () => {
    defineAnswer = () =>
      jsonResponse(400, {
        error: {
          code: "validationFailed",
          field: "name",
          message: "ข้อมูลไม่ถูกต้อง",
        },
      });
    await mountMembers();
    setInput(root.querySelector<HTMLInputElement>("#segment-name"), "VIP");
    setInput(
      root.querySelector<HTMLInputElement>("#segment-recency-days"),
      "30",
    );

    define();
    await settle();

    expect(writes).toHaveLength(1);
    expect(fieldLine("name")).toBe(
      sharedEn["input.invalid"].replace("{field}", table.en.fieldName),
    );
  });
});

describe("#4922 Consent retention days", () => {
  async function mountConsent(): Promise<HTMLElement> {
    dispose = render(
      () => (
        <>
          <Toast />
          <Consent {...makeProps()} />
        </>
      ),
      root,
    );
    await settle();
    const row = root.querySelector<HTMLElement>(
      '[data-retention="IDENTITY_VALUE"]',
    );
    expect(row).not.toBeNull();
    return row!;
  }

  it("CR1: retention 2147483648 (above i32) shows periodInvalid and sends nothing", async () => {
    const row = await mountConsent();
    setInput(row.querySelector("input"), "2147483648");
    row.querySelector<HTMLButtonElement>("button")!.click();
    await settle();

    expect(writes).toHaveLength(0);
    expect(row.querySelector(".error")?.textContent?.trim()).toBe(
      consentTable.en.periodInvalid,
    );
  });

  it("CR2: retention 30 is saved as today (PUT periodDays 30) and shows no error", async () => {
    const row = await mountConsent();
    setInput(row.querySelector("input"), "30");
    row.querySelector<HTMLButtonElement>("button")!.click();
    await settle();

    expect(writes).toHaveLength(1);
    expect(writes[0]!.method).toBe("PUT");
    expect(writes[0]!.pathname).toBe("/api/retention-policies/IDENTITY_VALUE");
    expect(writes[0]!.body).toEqual({ periodDays: 30, branch: "MAIN" });
    expect(row.querySelector(".error")).toBeNull();
    expect(root.querySelector(".toast")?.textContent?.trim()).toBe(
      consentTable.en.saved,
    );
  });
});
