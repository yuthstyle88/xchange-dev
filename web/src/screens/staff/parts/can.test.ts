// S-3 #575: `parts/can.ts` — the page decides what to show from `staff_me.permissions` only (SD:S2, SD:§4).
// Written before the implementation by the ticket author (claude/709ec749). Do not edit, delete or weaken.
//
// [required]: export function can(me: { permissions: readonly string[] } | undefined, permission: string): boolean
// Permissions are spelled as `staff_me.permissions` carries them (serde's default: the variant name, e.g.
// `BranchDayOperate`).
import { describe, expect, it } from "vitest";

type CanModule = {
  can: (me: { permissions: readonly string[] } | undefined, permission: string) => boolean;
};
const mods = import.meta.glob<CanModule>("./can.ts");

async function can(): Promise<CanModule["can"]> {
  const l = mods["./can.ts"];
  expect(l, "web/src/screens/staff/parts/can.ts exists").toBeDefined();
  return (await l!()).can;
}

describe("can(me, permission)", () => {
  it("C1 true exactly when staff_me.permissions holds the permission", async () => {
    const f = await can();
    const me = { permissions: ["AttendanceClock", "ShiftOperate"] };
    expect(f(me, "AttendanceClock")).toBe(true);
    expect(f(me, "ShiftOperate")).toBe(true);
    expect(f(me, "BranchDayOperate")).toBe(false);
  });

  it("C2 no actor yet (staff_me not read) → false", async () => {
    const f = await can();
    expect(f(undefined, "AttendanceClock")).toBe(false);
  });

  it("C3 reads only permissions: roles, names and branch give nothing", async () => {
    const f = await can();
    const me = {
      userId: "u",
      name: "BranchDayOperate",
      roles: [{ role: "BranchDayOperate", branchId: "MAIN" }],
      branchId: "BranchDayOperate",
      branchName: "BranchDayOperate",
      permissions: [] as string[],
    };
    expect(f(me, "BranchDayOperate")).toBe(false);
  });

  it("C4 exact match: no prefix, no case folding", async () => {
    const f = await can();
    const me = { permissions: ["CashManage"] };
    expect(f(me, "Cash")).toBe(false);
    expect(f(me, "cashmanage")).toBe(false);
    expect(f(me, "CashManageX")).toBe(false);
  });
});
