// V3-desk DK-C (#539): DeskPath, the one place /desk and /staff/desk differ.
// Design: docs/design/xchange-v3-wave3-desk.md §5.2 (DeskPath, OWNER_DESK, STAFF_DESK, deskPathFor), D-11.
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or
// weaken these tests; add your own in another file.
//
// These are the pure web tests the lane controller asked for first (2026-10-09). They need no API type.
// `quoteRoutes` is not pinned here: its shape belongs to the quote design (TX-3 #669), and its tests
// are added once that merges. The module is reached through import.meta.glob, so a missing
// desk-path.ts fails an assertion here instead of the whole file.
import { describe, expect, it } from "vitest";

// The shape of design §5.2, declared here so this file type-checks before desk-path.ts exists.
interface DeskPathShape {
  kind: "owner" | "staff";
  configView: "desk_config" | "staff_desk_config";
  rpc: "/api/rpc" | "/api/staff/rpc";
  upload: "/api/upload" | "/api/staff/upload";
  bookings: "/api/v2/bookings" | "/api/staff/bookings";
  fileUrl(key: string): string;
  devicesLink: boolean;
  usesDraft: boolean;
}
interface DeskPathModule {
  OWNER_DESK: DeskPathShape;
  STAFF_DESK: DeskPathShape;
  deskPathFor(pathname: string): DeskPathShape;
}

const mods = import.meta.glob<DeskPathModule>("./desk-path.ts");

async function load(): Promise<DeskPathModule> {
  const loader = mods["./desk-path.ts"];
  expect(loader, "web/src/screens/desk/desk-path.ts exists (design §5.2)").toBeDefined();
  return loader!();
}

// Each row is one path's required values (design §5.2 and the server routes on main):
// - rpc: api/src/presentation/http/rpc.rs:94-95
// - upload: api/src/presentation/http/files.rs:32,37
// - bookings: api/src/presentation/http/bookings.rs:78,84
// - files: api/src/application/compat/staff_view.rs:19,21
// - devicesLink: desk-devices.js:5 (/desk only)
// - usesDraft: X5 (staff only)
const EXPECTED = {
  owner: {
    kind: "owner",
    configView: "desk_config",
    rpc: "/api/rpc",
    upload: "/api/upload",
    bookings: "/api/v2/bookings",
    file: "/files/k1",
    devicesLink: true,
    usesDraft: false,
  },
  staff: {
    kind: "staff",
    configView: "staff_desk_config",
    rpc: "/api/staff/rpc",
    upload: "/api/staff/upload",
    bookings: "/api/staff/bookings",
    file: "/api/staff/file/k1",
    devicesLink: false,
    usesDraft: true,
  },
} as const;

function picture(p: DeskPathShape) {
  return {
    kind: p.kind,
    configView: p.configView,
    rpc: p.rpc,
    upload: p.upload,
    bookings: p.bookings,
    file: p.fileUrl("k1"),
    devicesLink: p.devicesLink,
    usesDraft: p.usesDraft,
  };
}

describe("DeskPath (DK-C #539, design §5.2)", () => {
  it("OWNER_DESK carries the owner's view, routes, file URL and flags", async () => {
    const { OWNER_DESK } = await load();
    expect(picture(OWNER_DESK)).toEqual(EXPECTED.owner);
  });

  it("STAFF_DESK carries the staff view, routes, file URL and flags", async () => {
    const { STAFF_DESK } = await load();
    expect(picture(STAFF_DESK)).toEqual(EXPECTED.staff);
  });

  it("the two paths differ in every route field, so no step can reach the other path's server", async () => {
    const { OWNER_DESK, STAFF_DESK } = await load();
    for (const field of ["configView", "rpc", "upload", "bookings"] as const) {
      expect(OWNER_DESK[field], field).not.toBe(STAFF_DESK[field]);
    }
    expect(OWNER_DESK.fileUrl("k1")).not.toBe(STAFF_DESK.fileUrl("k1"));
  });

  it("fileUrl puts the key after the path's file prefix, for any key", async () => {
    const { OWNER_DESK, STAFF_DESK } = await load();
    expect(OWNER_DESK.fileUrl("passport-7f3a")).toBe("/files/passport-7f3a");
    expect(STAFF_DESK.fileUrl("passport-7f3a")).toBe("/api/staff/file/passport-7f3a");
  });

  it("deskPathFor('/staff/desk') is STAFF_DESK", async () => {
    const { deskPathFor, STAFF_DESK } = await load();
    expect(deskPathFor("/staff/desk")).toBe(STAFF_DESK);
  });

  it("deskPathFor of any other pathname is OWNER_DESK", async () => {
    const { deskPathFor, OWNER_DESK } = await load();
    for (const p of ["/desk", "/", "/staff", "/staff/deskx", "/x/staff/desk", "/desk/staff/desk"]) {
      expect(deskPathFor(p), p).toBe(OWNER_DESK);
    }
  });
});
