// EX-9 #1147: `walkPages` is the export walker, so every read it makes carries `purpose: 'export'`.
// Written before the implementation by the up-front test writer (claude/709ec749, on the BE lane controller
// claude/ad05bf8d's order). Do not edit, delete or weaken; add tests in a file of your own.
//
// Ticket §7 [required]: `walkPages` passes `purpose: 'export'` on every read; its other params are unchanged
// (`branch`, `filter`, `limit: 100`, `before` from page 2 on).
import { describe, expect, it } from "vitest";
import type { ApiClient } from "../../../shared/api/client";
import { walkPages } from "./csv";

type Row = { id: string };

function rows(prefix: string, n: number): Row[] {
  return Array.from({ length: n }, (_, i) => ({ id: `${prefix}-${i + 1}` }));
}

/** A fake ApiClient whose `page` answers `pages` in turn and records the params of each call. */
function fake(pages: Row[][]) {
  const calls: Record<string, unknown>[] = [];
  const api = {
    page: async (_view: string, params: Record<string, unknown>) => {
      calls.push({ ...params });
      return pages.shift() ?? [];
    },
  } as unknown as ApiClient;
  return { api, calls };
}

describe("EX-9 walkPages marks every read as an export", () => {
  it("a three-page walk sends purpose: export on each read, with its other params unchanged", async () => {
    const p1 = rows("a", 100);
    const p2 = rows("b", 100);
    const p3 = rows("c", 5);
    const { api, calls } = fake([p1, p2, p3]);
    const all = await walkPages<Row>(api, "admin_transactions", {
      branch: "B1",
      filter: { from: "2026-10-01" },
    });
    expect(all).toHaveLength(205);
    expect(calls).toEqual([
      {
        branch: "B1",
        filter: { from: "2026-10-01" },
        limit: 100,
        before: undefined,
        purpose: "export",
      },
      {
        branch: "B1",
        filter: { from: "2026-10-01" },
        limit: 100,
        before: "a-100",
        purpose: "export",
      },
      {
        branch: "B1",
        filter: { from: "2026-10-01" },
        limit: 100,
        before: "b-100",
        purpose: "export",
      },
    ]);
  });

  it("a one-page walk is an export too", async () => {
    const { api, calls } = fake([rows("a", 3)]);
    await walkPages<Row>(api, "admin_transactions", {
      branch: "MAIN",
      filter: {},
    });
    expect(calls).toHaveLength(1);
    expect(calls[0]!["purpose"]).toBe("export");
  });
});
