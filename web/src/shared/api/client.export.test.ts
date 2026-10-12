// EX-9 #1147: `ApiClient.page` carries `purpose=export` when asked, and nothing new otherwise.
// Written before the implementation by the up-front test writer (claude/709ec749, on the BE lane controller
// claude/ad05bf8d's order). Do not edit, delete or weaken; add tests in a file of your own.
//
// Ticket §7 [required]: `PageParams.purpose?: 'export'`; `page()` sets the query key `purpose` when it is present.
// The params are passed through a cast so this file compiles before `purpose` joins `PageParams`.
import { describe, expect, it, vi } from "vitest";
import { ApiClient } from "./client";

type PageParams = Parameters<ApiClient["page"]>[1];

function client() {
  const urls: string[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL) => {
    urls.push(String(input));
    return new Response(
      JSON.stringify({ version: 1, changed: true, data: [] }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      },
    );
  });
  return { api: new ApiClient(fetchFn as unknown as typeof fetch), urls };
}

function query(url: string): URLSearchParams {
  return new URLSearchParams(url.split("?")[1] ?? "");
}

describe("EX-9 client.page and purpose", () => {
  it("with purpose: export, the URL has purpose=export next to the other keys", async () => {
    const { api, urls } = client();
    await api.page("admin_transactions", {
      branch: "B1",
      limit: 100,
      before: "XC-1",
      filter: { from: "2026-10-01" },
      purpose: "export",
    } as unknown as PageParams);
    expect(urls).toHaveLength(1);
    const q = query(urls[0]!);
    expect(q.get("purpose")).toBe("export");
    expect(q.getAll("purpose")).toEqual(["export"]);
    expect(q.get("branch")).toBe("B1");
    expect(q.get("limit")).toBe("100");
    expect(q.get("before")).toBe("XC-1");
    expect(q.get("from")).toBe("2026-10-01");
  });

  it("without purpose, the URL carries no purpose key (unchanged)", async () => {
    const { api, urls } = client();
    await api.page("admin_transactions", {
      branch: "B1",
      limit: 100,
      filter: { from: "2026-10-01" },
    });
    expect(query(urls[0]!).has("purpose")).toBe(false);
    expect([...query(urls[0]!).keys()].sort()).toEqual([
      "branch",
      "from",
      "limit",
    ]);
  });
});
