// RF-4: the customer step marks the field a `reserveCustomer` refusal names
// (docs/design/rpc-refusal-field.md §4 rows R26–R33, decisions RF-D7, RF-D8).
// Written before the implementation by the sub-controller (RF-0). Do not edit, delete or weaken.
//
// The server answers `{"error":"<v2 sentence>","field":"<key>"}` (RF-D1); `ApiClient` reads `field`
// (RF-3); the step passes its five labels to `fieldError` and shows the sentence under that field.
// Anything else (no field, `mode`, an unknown key, 5xx, network, any refusal while skipping) stays on
// the form line, as `worded.test.tsx` holds.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { setLang } from "../../../../shared/i18n";
import { th as sharedTh } from "../../../../shared/i18n/th";
import { ApiClient } from "../../../../shared/api/client";
import { step } from "./step";
import * as texts from "./i18n";

const PATH_OWNER = {
  kind: "owner",
  rpc: "/api/rpc",
  usesDraft: false,
} as const;
const BAD_TEXT = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
const BAD_DATE = "วันที่ไม่ถูกต้อง";
const DUMMY_FILE = new File([new Uint8Array([1, 2, 3])], "passport.png", {
  type: "image/png",
});

function refusing(status: number, body: unknown): ApiClient {
  const fetchFn = vi.fn(
    async () =>
      new Response(JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
      }),
  );
  return new ApiClient(fetchFn as unknown as typeof fetch);
}

function createStubStore(api: ApiClient) {
  const [passport, setPassportSignal] = createSignal<File | undefined>(
    DUMMY_FILE,
  );
  const [fields, setFieldsSignal] = createSignal({
    name: "ALICE SMITH",
    passportNo: "A12345678",
    nationality: "USA",
    birthDate: "1995-05-15",
    passportExpiry: "2030-05-15",
  });
  const [identityChecked, setIdentityCheckedSignal] = createSignal(true);
  const [customerId, setCustomerIdSignal] = createSignal("");
  const [staffDraftId, setStaffDraftIdSignal] = createSignal("");
  return {
    api,
    path: PATH_OWNER,
    branch: () => "XC-B01",
    type: () => "BUY",
    currency: () => "USD",
    amount: () => "500.00",
    step: () => 1,
    config: () => ({ shiftOpen: true }),
    passport,
    setPassport: vi.fn((f: File | undefined) => setPassportSignal(f)),
    fields,
    setField: vi.fn((k: string, v: string) =>
      setFieldsSignal({ ...fields(), [k]: v }),
    ),
    editedByUser: () => new Set<string>(),
    identityChecked,
    setIdentityChecked: vi.fn((v: boolean) => setIdentityCheckedSignal(v)),
    customerId,
    setCustomerId: vi.fn((v: string) => setCustomerIdSignal(v)),
    // C1 (walkin-queue.md §12.3): no signed-in teller id, so the step reads no held ticket.
    meId: () => "",
    heldBooking: () => undefined,
    setHeldBooking: vi.fn(),
    staffDraftId,
    setStaffDraftId: vi.fn((v: string) => setStaffDraftIdSignal(v)),
    entry: () => undefined,
    go: vi.fn(),
  };
}

describe("screens/desk/steps/customer: a refusal that names a field (RF-4)", () => {
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

  function mount(api: ApiClient) {
    const store = createStubStore(api);
    dispose = render(() => step.component({ store: store as any }), host);
    return store;
  }

  const control = (name: string) => {
    const el = host.querySelector<HTMLInputElement | HTMLSelectElement>(
      `[name="${name}"]`,
    );
    expect(el, name).not.toBeNull();
    return el!;
  };
  /** The message under the field `name`, or undefined. */
  const fieldMessage = (name: string) =>
    control(name).closest("label")?.querySelector('[role="alert"]')
      ?.textContent ?? undefined;
  /** The form-level line, or undefined. */
  const formLine = () =>
    host.querySelector('form > p.desk-error[role="alert"]')?.textContent ??
    undefined;
  const allFieldMessages = () =>
    ["name", "passportNo", "nationality", "birthDate", "passportExpiry"]
      .map(fieldMessage)
      .filter(Boolean);

  const submit = () =>
    host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
  const skip = () =>
    [...host.querySelectorAll("button")]
      .find((b) => b.textContent?.includes(texts.th.skip))!
      .click();

  // R26
  it("passport_refusal_marks_passport", async () => {
    mount(refusing(400, { error: BAD_TEXT, field: "passportNo" }));
    submit();
    await vi.waitFor(() => expect(fieldMessage("passportNo")).toBe(BAD_TEXT));
    expect(formLine()).toBeUndefined();
    expect(allFieldMessages()).toEqual([BAD_TEXT]);
  });

  // R27
  it("each_text_field_is_marked", async () => {
    for (const [field, sentence] of [
      ["name", BAD_TEXT],
      ["birthDate", BAD_DATE],
      ["passportExpiry", BAD_DATE],
    ] as const) {
      mount(refusing(400, { error: sentence, field }));
      submit();
      await vi.waitFor(() => expect(fieldMessage(field), field).toBe(sentence));
      expect(formLine(), field).toBeUndefined();
      expect(allFieldMessages(), field).toEqual([sentence]);
      dispose?.();
      dispose = undefined;
      host.replaceChildren();
    }
  });

  // R28
  it("nationality_refusal_marks_the_select", async () => {
    mount(refusing(400, { error: "สัญชาติไม่ถูกต้อง", field: "nationality" }));
    submit();
    await vi.waitFor(() =>
      expect(fieldMessage("nationality")).toBe("สัญชาติไม่ถูกต้อง"),
    );
    expect(formLine()).toBeUndefined();
    // picking another nationality clears it, as typing clears the other fields
    const select = control("nationality") as HTMLSelectElement;
    select.value = "DEU";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() => expect(fieldMessage("nationality")).toBeUndefined());
  });

  // R29
  it("unlabelled_field_stays_at_form_level", async () => {
    for (const body of [
      { error: "กรุณาเลือกประเภทรายการ", field: "mode" },
      { error: "unknown key refusal", field: "branchId" },
      { error: "no field refusal" },
    ]) {
      mount(refusing(400, body));
      submit();
      await vi.waitFor(() =>
        expect(formLine(), JSON.stringify(body)).toBe(body.error),
      );
      expect(allFieldMessages(), JSON.stringify(body)).toEqual([]);
      dispose?.();
      dispose = undefined;
      host.replaceChildren();
    }
  });

  // R30
  it("typing_clears_the_marked_field", async () => {
    mount(refusing(400, { error: BAD_TEXT, field: "passportNo" }));
    submit();
    await vi.waitFor(() => expect(fieldMessage("passportNo")).toBe(BAD_TEXT));
    const input = control("passportNo") as HTMLInputElement;
    input.value = "B7654321";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.waitFor(() => expect(fieldMessage("passportNo")).toBeUndefined());
  });

  // R31 (RF-D8)
  it("skip_refusal_stays_at_form_level", async () => {
    mount(refusing(400, { error: BAD_TEXT, field: "passportNo" }));
    skip();
    await vi.waitFor(() => {
      expect(formLine()).toContain("ข้ามไม่สำเร็จ:");
      expect(formLine()).toContain(BAD_TEXT);
    });
    expect(allFieldMessages()).toEqual([]);
  });

  // R32
  it("server_and_network_failures_stay_at_form_level", async () => {
    mount(refusing(500, { error: "boom", field: "passportNo" }));
    submit();
    await vi.waitFor(() =>
      expect(formLine()).toBe(sharedTh["error.serverError"]),
    );
    expect(allFieldMessages()).toEqual([]);
    dispose?.();
    dispose = undefined;
    host.replaceChildren();

    const offline = vi.fn(async () => {
      throw new TypeError("Failed to fetch");
    });
    mount(new ApiClient(offline as unknown as typeof fetch));
    submit();
    await vi.waitFor(() => expect(formLine()).toBe(sharedTh["error.network"]));
    expect(allFieldMessages()).toEqual([]);
  });

  // R33 (W10 (4): the server's sentence, not a translation)
  it("field_message_is_the_server_sentence_in_en", async () => {
    setLang("en");
    mount(refusing(400, { error: BAD_TEXT, field: "passportNo" }));
    submit();
    await vi.waitFor(() => expect(fieldMessage("passportNo")).toBe(BAD_TEXT));
    expect(formLine()).toBeUndefined();
  });
});
