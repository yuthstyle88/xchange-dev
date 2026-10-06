// Records how the v2 server builds its pages and picks its static files, for
// `api/tests/pages_test.rs`.
//
// Needs `pages-harness.mjs` beside it — these lines of the deployed bundle's
// `server.mjs` (`xchange-online@3ffc5a5`), unchanged, wrapped as shown:
//
//   import fs2 from "node:fs";
//   import path2 from "node:path";
//   var AUTH = true, DEVICES2 = true;               // v3 has every v2 module on
//   var PUBLIC_DIR = process.env.PAGES_PUBLIC_DIR;
//   <lines 5545-5550>                               // htmlResponse
//   <lines 8196-8233>                               // BAR, DESK_SCRIPTS, ADMIN_SCRIPTS, withScripts, shell, TYPES, staticFile
//   function staffDesk(index_html_default, e2) {
//   <lines 7416-7418>                               // the /staff/desk variant
//     return html;
//   }
//   export { htmlResponse, withScripts, shell, staticFile, staffDesk, BAR, DESK_SCRIPTS, ADMIN_SCRIPTS };
//
// The bundle is not in this repository, so this script cannot run in CI; its
// output is checked in and is what the tests read. The pages it works on are
// the small ones under `api/tests/fixtures/pages/` — never v2's own.
//
//   node gen_pages.mjs > ../../tests/fixtures/pages/v2_page_cases.json

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const here = path.dirname(fileURLToPath(import.meta.url));
const fixtures = path.resolve(here, "../../tests/fixtures/pages");

// ---- one directory, in a child process (PUBLIC_DIR is read when the harness loads)
if (process.argv[2]) {
  const dir = process.argv[2];
  const h = await import("./pages-harness.mjs");
  const ORIGIN = "http://xc.test";
  const page = (name) => fs.readFileSync(path.join(fixtures, dir, name + ".html"), "utf8");
  const has = (name) => fs.existsSync(path.join(fixtures, dir, name + ".html"));

  // The route table of server.mjs:8143-8188, the page rows only.
  //   kind "branch": htmlResponse (branch injected), then the scripts
  //   kind "shell":  the file as it is, then the scripts
  const ROUTES = [
    ["/", "user", "branch", "admin", h.ADMIN_SCRIPTS],
    ["/desk", "user", "branch", "desk", h.DESK_SCRIPTS],
    ["/receipt-sample", "user", "branch", "receipt-sample", []],
    ["/staff", "public", "shell", "staff", []],
    ["/staff/report", "public", "shell", "staff-report", []],
    ["/devices", "user", "shell", "devices", h.BAR],
    ["/login", "public", "shell", "login", []],
    ["/kiosk", "public", "shell", "kiosk", []],
    ["/kiosk-queue.html", "user", "shell", "kiosk-queue", h.BAR],
    ["/kiosk-currencies.html", "user", "shell", "kiosk-currencies", h.BAR],
    ["/staff-guide", "user", "shell", "staff-guide", h.BAR],
    ["/staff-guide.html", "user", "shell", "staff-guide", h.BAR],
  ];
  // "none" | "owner" | "staff:ROLE:BRANCH" | "owner+staff:ROLE:BRANCH"
  const sessions = (v) => {
    const owner = v.startsWith("owner");
    const s = v.includes("staff:") ? v.slice(v.indexOf("staff:")).split(":") : null;
    return { owner, staff: s && { role: s[1], branchId: s[2] } };
  };
  const answer = async (res) => {
    res = await res;
    const headers = {};
    res.headers.forEach((v, k) => (headers[k] = v));
    return { status: res.status, headers, body: await res.text() };
  };
  async function get(pathname, query, visitor) {
    const url = new URL(ORIGIN + pathname + (query ? "?" + query : ""));
    const who = sessions(visitor);
    // server.mjs:7522-7526 with 7414-7419 — the staff session alone decides
    if (pathname === "/staff/desk") {
      if (!who.staff || who.staff.role !== "FRONT") return { status: 302, location: "/staff" };
      const inner = new Response(h.staffDesk(page("desk"), who.staff), {
        headers: { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "private, no-store" },
      });
      return answer(h.withScripts(inner, h.DESK_SCRIPTS));
    }
    const route = ROUTES.find((r) => r[0] === pathname);
    const [, access, kind, name, scripts] = route;
    // server.mjs:8276-8284
    const actor = who.owner ? { kind: "owner" } : who.staff ? { kind: "staff", ...who.staff } : null;
    if (access !== "public") {
      if (!actor) return { status: 303, location: "/login" };
      if (actor.kind === "staff" && url.searchParams.get("branch") !== actor.branchId) url.searchParams.set("branch", actor.branchId);
    }
    if (kind === "shell") return answer(h.withScripts(h.shell(name), scripts));
    const req = new Request(url, { headers: { "oai-authenticated-user-id": "x" } });
    return answer(h.withScripts(h.htmlResponse(req, page(name)), scripts));
  }

  const pages = [];
  const add = async (label, pathname, query, visitor) => {
    pages.push({ label, dir, path: pathname, query, visitor, ...(await get(pathname, query, visitor)) });
  };

  if (dir === "web") {
    for (const [p] of ROUTES) await add("the owner asks", p, "", "owner");
    for (const [p, access] of ROUTES) await add(access === "public" ? "nobody is signed in, and need not be" : "nobody is signed in", p, "", "none");
    for (const [p] of ROUTES) await add("a staff member asks", p, "", "staff:FRONT:XC-B01");
    // the branch the page is told
    for (const p of ["/", "/desk", "/receipt-sample"]) {
      await add("the owner names a branch", p, "branch=XC-B02", "owner");
      await add("an empty branch is MAIN", p, "branch=", "owner");
      await add("other parameters do not matter", p, "x=1&branch=XC-B07&y=2", "owner");
      await add("the first of two branches counts", p, "branch=AAA&branch=BBB", "owner");
      await add("a branch is percent-decoded", p, "branch=XC%2DB03", "owner");
      await add("seventy characters is the longest branch", p, "branch=" + "a".repeat(70), "owner");
      await add("seventy-one characters is refused", p, "branch=" + "a".repeat(71), "owner");
      await add("a plus sign is a space, and a space is refused", p, "branch=XC+B", "owner");
      await add("an underscore is refused", p, "branch=XC_B", "owner");
      await add("a quote is refused", p, "branch=%22%3E", "owner");
      await add("a Thai branch is refused", p, "branch=%E0%B8%AA%E0%B8%B2%E0%B8%82%E0%B8%B2", "owner");
      await add("a staff member gets the own branch whatever is named", p, "branch=XC-B02", "staff:FRONT:XC-B01");
      await add("a staff member gets the own branch when a bad one is named", p, "branch=%22%3E", "staff:FINANCE:XC-B05");
      await add("a staff member gets the own branch when two are named", p, "branch=AAA&branch=BBB", "staff:MANAGER:XC-B09");
      await add("an owner session wins over a staff session", p, "branch=XC-B02", "owner+staff:FRONT:XC-B01");
      await add("nobody is signed in and the branch is bad: the sign-in page comes first", p, "branch=%22%3E", "none");
    }
    // /staff/desk
    await add("a Front staff member gets the desk variant", "/staff/desk", "", "staff:FRONT:XC-B01");
    await add("the variant ignores the branch in the address", "/staff/desk", "branch=XC-B02", "staff:FRONT:XC-B04");
    await add("a Front staff session counts even beside an owner session", "/staff/desk", "", "owner+staff:FRONT:XC-B06");
    for (const role of ["BACKEND", "FINANCE", "MANAGER", "RATES", "SYSTEM"]) await add("only Front gets the desk variant", "/staff/desk", "", "staff:" + role + ":XC-B01");
    await add("the owner has no staff session", "/staff/desk", "", "owner");
    await add("nobody is signed in", "/staff/desk", "", "none");
  } else {
    await add("a page without a head tag and without closing tags", "/", "branch=XC-B02", "owner");
    await add("a desk page without a closing body", "/desk", "", "owner");
    await add("a desk variant without a closing body", "/staff/desk", "", "staff:FRONT:XC-B01");
  }

  // static files: what `handle` does last (server.mjs:8287-8291)
  const statics = [];
  if (dir === "web") {
    const raw = [
      "/app.js", "/style.css", "/favicon.svg", "/logo.jpeg", "/photo.jpg", "/dot.png", "/note.txt", "/data.json",
      "/fonts/thai-0a1b2c3d.woff2", "/fonts/OFL.txt", "/vendor/lib-1.2.3.min.js",
      "/two%20words.txt", "/%E0%B9%84%E0%B8%97%E0%B8%A2.txt", "/fonts%2FOFL.txt", "/%61pp.js",
      "/admin.html", "/desk.html", "/login.html", "/staff.html", "/kiosk.html", "/receipt-sample.html", "/devices.html",
      "/UPPER.JS", "/blob.bin", "/LICENSE", "/.hidden", "/.js", "/app.js.map", "/folder.js", "/folder.js/readme.md",
      "/fonts", "/fonts/", "/missing.js", "/app", "/favicon.ico", "/style.CSS",
    ];
    for (const p of raw) {
      const pathname = new URL(ORIGIN + p).pathname;
      const res = h.staticFile(pathname);
      if (!res) { statics.push({ dir, path: p, status: 404 }); continue; }
      const headers = {};
      res.headers.forEach((v, k) => (headers[k] = v));
      const file = path.relative(path.join(fixtures, dir), path.resolve(path.join(fixtures, dir), "." + decodeURIComponent(pathname)));
      statics.push({ dir, path: p, status: 200, headers, file });
      await res.body.cancel();
    }
  }
  process.stdout.write(JSON.stringify({ pages, statics }));
  process.exit(0);
}

// ---- main: both directories, one file
const out = { source: "xchange-online@3ffc5a5 server.mjs:5545-5550,7416-7418,8196-8233", pages: [], statics: [] };
for (const dir of ["web", "web-bare"]) {
  const r = spawnSync(process.execPath, [fileURLToPath(import.meta.url), dir], {
    env: { ...process.env, PAGES_PUBLIC_DIR: path.join(fixtures, dir) },
    maxBuffer: 1 << 26,
  });
  if (r.status !== 0) { process.stderr.write(r.stderr); process.exit(1); }
  const part = JSON.parse(r.stdout);
  out.pages.push(...part.pages);
  out.statics.push(...part.statics);
}
process.stdout.write(JSON.stringify(out, null, 1) + "\n");
