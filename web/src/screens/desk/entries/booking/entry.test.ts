// #543 Part 3, DK-I, booking half: the desk opened on `?booking=<id>` (design docs/design/xchange-v3-wave3-desk.md §3.6
// B1–B3, D-13, §5.3 `DeskEntryState` / `DeskEntry`, §6 row BookingEntry). Runs on both OWNER_DESK and STAFF_DESK.
// Written before the implementation by the test-writer (claude/709ec749, on the FE lane controller claude/5edf7bb5's
// order). Do not edit, delete or weaken these tests; add your own elsewhere.
//
// [required] by these tests (the module is reached through import.meta.glob, so a missing file fails an assertion):
// - `entries/booking/entry.ts` exports `entry: DeskEntry` with `param: 'booking'` and `load(store, id, api)`.
// - `load` asks `POST <store.path.bookings>` `{ action: 'get', payload: { branchId: store.branch(), id } }` through the
//   `api` it is given (an `ApiClient`), and resolves with a `DeskEntryState` of `kind: 'booking'` and that `id`.
// - The booking's answer is v2's booking object (`api/src/presentation/dto/booking.rs` `booking_json` +
//   `customerId`). No generated TS type exists for this compat route, so the fixture is declared here with that file's
//   field names.
// - CONFIRMED (B1): `type`, `currency` and `amount` go through the store's setters (`amount` as the server's number
//   written as a string, no arithmetic); the rate is **not** set (D-13: the quote uses the published rate). `locked` is
//   true (B2). `identityLabel` is v2's `ตรวจตัวตนลูกค้าตรงกับผู้จอง <name> แล้ว` (B2). `refusal()` is v2's
//   `กรุณายืนยันตัวตนว่าตรงกับผู้จองก่อนบันทึก` until `setIdentityConfirmed(true)` (B3). `payloadExtras()` carries the
//   booking id as `bookingId` or `onlineBookingId` (both read by `save.ts` `createBody`).
// - Any other status (B1): v2's `การจองยังไม่ยืนยัน หมดอายุ หรือใช้ไปแล้ว` is the banner and the refusal; no setter runs.
// - A refused `get` (4xx): the server's sentence is the banner and the refusal; no setter runs.
// NOT pinned here, on purpose:
// - B2's "the whole panel is disabled while loading or on error": `DeskEntryState` has no field for it and the panel is
//   `DeskPage.tsx`'s (DK-C's root file, outside #543's scope). See the preparer's comment.
// - The banner's `เรท <rate>` part: D-13 says a booking does not set the rate, so whether the banner still shows the
//   booking's rate is the designer's call. The test pins only the id and the name.
// - The kiosk half (`entries/kiosk/`): design O-1 is open and #543 Part 3 §7 says it must be decided first.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { ApiClient } from "../../../../shared/api/client";

type EntryState = {
  kind: string;
  id: string;
  banner: () => string;
  locked: boolean;
  identityLabel?: string;
  identityConfirmed: () => boolean;
  setIdentityConfirmed(v: boolean): void;
  payloadExtras(): Record<string, string | boolean>;
  refusal(): string | undefined;
};
type EntryModule = { entry: { param: string; load(store: any, id: string, api: ApiClient): Promise<EntryState> } };
const entryMods = import.meta.glob<EntryModule>("./entry.ts");
const SOURCES = import.meta.glob<string>(["./*.ts", "./*.tsx", "!./*.test.ts", "!./*.test.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
});

async function loadEntry() {
  const l = entryMods["./entry.ts"];
  expect(l, "web/src/screens/desk/entries/booking/entry.ts exists").toBeDefined();
  return (await l!()).entry;
}

const NOT_CONFIRMED = "การจองยังไม่ยืนยัน หมดอายุ หรือใช้ไปแล้ว";
const CONFIRM_IDENTITY = "กรุณายืนยันตัวตนว่าตรงกับผู้จองก่อนบันทึก";

/** v2's booking object, as `booking_json` + `customerId` answer it (`api/src/presentation/dto/booking.rs:51-79`). */
type BookingAnswer = {
  id: string;
  branchId: string;
  name: string;
  phone: string;
  member: boolean;
  type: "BUY" | "SELL";
  currency: string;
  amount: number;
  rate: number;
  status: string;
  createdAt: string;
  expiresAt: string | null;
  transactionRef: string | null;
  customerId: string | null;
};

const booking = (over: Partial<BookingAnswer> = {}): BookingAnswer => ({
  id: "BK-0042",
  branchId: "MAIN",
  name: "JANE ROE",
  phone: "0812341688",
  member: false,
  type: "SELL",
  currency: "JPY",
  amount: 10000,
  rate: 0.2195,
  status: "CONFIRMED",
  createdAt: "2026-10-11T02:00:00.000Z",
  expiresAt: "2026-10-11T10:00:00.000Z",
  transactionRef: null,
  customerId: null,
  ...over,
});

const PATHS = {
  OWNER_DESK: { kind: "owner", bookings: "/api/bookings" },
  STAFF_DESK: { kind: "staff", bookings: "/api/staff/bookings" },
} as const;

type Call = { url: string; method: string; body: any };
let calls: Call[] = [];

function serve(reply: { status: number; body: unknown }) {
  calls = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      calls.push({ url, method: String(init?.method ?? "GET").toUpperCase(), body: init?.body ? JSON.parse(String(init.body)) : null });
      return new Response(JSON.stringify(reply.body), { status: reply.status, headers: { "content-type": "application/json" } });
    }),
  );
}

function stubStore(path: (typeof PATHS)[keyof typeof PATHS]) {
  const [type, setType] = createSignal("BUY");
  const [currency, setCurrency] = createSignal("USD");
  const [amount, setAmount] = createSignal("");
  return {
    path,
    branch: () => "MAIN",
    type,
    currency,
    amount,
    setType: vi.fn((v: string) => setType(v)),
    setCurrency: vi.fn((v: string) => setCurrency(v)),
    setAmount: vi.fn((v: string) => setAmount(v)),
    setReferenceRate: vi.fn(),
    setBooking: vi.fn(),
  };
}

beforeEach(() => {
  calls = [];
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe("entries/booking: the module", () => {
  it("I0 exports the booking entry", async () => {
    const entry = await loadEntry();
    expect(entry.param).toBe("booking");
    expect(typeof entry.load).toBe("function");
  });

  it("I0b no money arithmetic and no fetch( in the entry's sources (W7)", () => {
    expect(Object.keys(SOURCES)).toContain("./entry.ts");
    for (const [file, src] of Object.entries(SOURCES)) {
      expect(src, file).not.toMatch(/Number\(|parseFloat|parseInt|Math\.round|toFixed|\* *100/);
      expect(src, file).not.toMatch(/\bfetch\(/);
    }
  });
});

for (const which of ["OWNER_DESK", "STAFF_DESK"] as const) {
  describe(`entries/booking on ${which}`, () => {
    it(`I1 asks POST ${PATHS[which].bookings} {action:'get', payload:{branchId, id}} through the api it is given`, async () => {
      serve({ status: 200, body: booking() });
      const entry = await loadEntry();
      const store = stubStore(PATHS[which]);
      await entry.load(store, "BK-0042", new ApiClient());
      const gets = calls.filter((c) => c.method === "POST" && c.url.split("?")[0] === PATHS[which].bookings);
      expect(gets).toHaveLength(1);
      expect(gets[0]!.body).toEqual({ action: "get", payload: { branchId: "MAIN", id: "BK-0042" } });
    });

    it("I2 CONFIRMED: type, currency and amount through the setters; the rate is not set (D-13); locked; banner names the booking", async () => {
      serve({ status: 200, body: booking() });
      const entry = await loadEntry();
      const store = stubStore(PATHS[which]);
      const state = await entry.load(store, "BK-0042", new ApiClient());
      expect(state.kind).toBe("booking");
      expect(state.id).toBe("BK-0042");
      expect(store.setType).toHaveBeenCalledWith("SELL");
      expect(store.setCurrency).toHaveBeenCalledWith("JPY");
      expect(store.setAmount).toHaveBeenCalledWith("10000");
      expect(store.setReferenceRate).not.toHaveBeenCalled();
      expect(store.setBooking, "no client booking reference (D-10)").not.toHaveBeenCalled();
      expect(state.locked).toBe(true);
      expect(state.banner()).toContain("BK-0042");
      expect(state.banner()).toContain("JANE ROE");
    });

    it("I3 CONFIRMED: v2's identity checkbox label and the save refusal until it is confirmed (B2, B3)", async () => {
      serve({ status: 200, body: booking() });
      const entry = await loadEntry();
      const state = await entry.load(stubStore(PATHS[which]), "BK-0042", new ApiClient());
      expect(state.identityLabel).toBe("ตรวจตัวตนลูกค้าตรงกับผู้จอง JANE ROE แล้ว");
      expect(state.identityConfirmed()).toBe(false);
      expect(state.refusal()).toBe(CONFIRM_IDENTITY);
      state.setIdentityConfirmed(true);
      expect(state.identityConfirmed()).toBe(true);
      expect(state.refusal()).toBeUndefined();
    });

    it("I4 CONFIRMED: the save carries the booking id (bookingId or onlineBookingId)", async () => {
      serve({ status: 200, body: booking() });
      const entry = await loadEntry();
      const state = await entry.load(stubStore(PATHS[which]), "BK-0042", new ApiClient());
      state.setIdentityConfirmed(true);
      const x = state.payloadExtras();
      expect(x.bookingId ?? x.onlineBookingId).toBe("BK-0042");
    });

    it("I5 a booking that is not CONFIRMED: v2's sentence is the banner and the refusal; no setter runs", async () => {
      for (const status of ["PENDING", "EXPIRED", "USED", "CANCELLED"]) {
        serve({ status: 200, body: booking({ status }) });
        const entry = await loadEntry();
        const store = stubStore(PATHS[which]);
        const state = await entry.load(store, "BK-0042", new ApiClient());
        expect(state.banner(), status).toBe(NOT_CONFIRMED);
        expect(state.refusal(), status).toBe(NOT_CONFIRMED);
        state.setIdentityConfirmed(true);
        expect(state.refusal(), `${status}: confirming identity does not lift it`).toBe(NOT_CONFIRMED);
        expect(store.setType, status).not.toHaveBeenCalled();
        expect(store.setAmount, status).not.toHaveBeenCalled();
      }
    });

    it("I6 a refused get (4xx with v2's string error): the server's sentence is the banner and the refusal; no setter runs", async () => {
      serve({ status: 400, body: { error: "ไม่พบการจอง" } });
      const entry = await loadEntry();
      const store = stubStore(PATHS[which]);
      const state = await entry.load(store, "BK-9999", new ApiClient());
      expect(state.banner()).toBe("ไม่พบการจอง");
      expect(state.refusal()).toBe("ไม่พบการจอง");
      expect(store.setType).not.toHaveBeenCalled();
    });
  });
}
