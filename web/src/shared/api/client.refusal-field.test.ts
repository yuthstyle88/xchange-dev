// RF-3: ApiClient keeps the `field` an RPC refusal names (docs/design/rpc-refusal-field.md §4 rows
// R22–R25, decisions RF-D1, RF-D7). Written before the implementation by the sub-controller (RF-0).
// Do not edit, delete or weaken.
//
// The server answers a refusal of `reserveCustomer` as `{"error":"<v2 sentence>","field":"<key>"}`
// (RF-D1). `parseError` reads a string `error` as `ApiError{code:'refused'}` and, from now on, also
// takes a non-empty string `field` beside it. An object `error` (v3's) is read as before.
import { describe, expect, it } from "vitest";
import { ApiClient, ApiError, type Fetch } from "./client";

const BAD_TEXT = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";

function client(status: number, body: unknown): ApiClient {
  const text = typeof body === "string" ? body : JSON.stringify(body);
  const fetchFn: Fetch = async () =>
    new Response(text, {
      status,
      headers: { "Content-Type": "application/json" },
    });
  return new ApiClient(fetchFn);
}

async function refusalOf(status: number, body: unknown): Promise<ApiError> {
  try {
    await client(status, body).post("/api/rpc", {
      action: "reserveCustomer",
      payload: {},
    });
  } catch (e) {
    expect(e).toBeInstanceOf(ApiError);
    return e as ApiError;
  }
  throw new Error("expected a refusal");
}

describe("ApiClient: a string refusal that names a field", () => {
  // R22
  it("string_error_with_field", async () => {
    const e = await refusalOf(400, { error: BAD_TEXT, field: "passportNo" });
    expect(e.code).toBe("refused");
    expect(e.message).toBe(BAD_TEXT);
    expect(e.field).toBe("passportNo");
    expect(e.status).toBe(400);
  });

  // R23
  it("string_error_without_field_is_unchanged", async () => {
    const e = await refusalOf(400, { error: BAD_TEXT });
    expect(e.code).toBe("refused");
    expect(e.message).toBe(BAD_TEXT);
    expect(e.field).toBeUndefined();
    expect(e.reason).toBeUndefined();
    expect(e.status).toBe(400);
  });

  // R24
  it("non_string_or_empty_field_is_ignored", async () => {
    for (const field of [
      7,
      "",
      null,
      true,
      { key: "passportNo" },
      ["passportNo"],
    ]) {
      const e = await refusalOf(400, { error: BAD_TEXT, field });
      expect(e.code).toBe("refused");
      expect(e.message).toBe(BAD_TEXT);
      expect(e.field, JSON.stringify(field)).toBeUndefined();
    }
  });

  // R25
  it("object_error_is_unchanged", async () => {
    const e = await refusalOf(400, {
      error: { code: "validationFailed", field: "x", message: "m" },
    });
    expect(e.code).toBe("validationFailed");
    expect(e.field).toBe("x");
    expect(e.message).toBe("m");
    // a sibling `field` never overrides v3's own
    const both = await refusalOf(400, {
      error: { code: "validationFailed", field: "x", message: "m" },
      field: "passportNo",
    });
    expect(both.field).toBe("x");
  });
});
