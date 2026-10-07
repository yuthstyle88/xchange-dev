// Specification tests for the page-name clash in `extract-v2-pages.mjs`
// (#261; #228 item 6(a)).
//
// Written before the implementation. Do not edit, delete or weaken.
//
//   node --test api/tools/extract-v2-pages-clash.test.mjs     (Node 20 or newer)
//
// The script writes `admin.html`, `desk.html` and `receipt-sample.html` at the
// top of `<web-dir>` and copies `<bundle-dir>/public/` beside them. Ticket #80
// §8.6: anything at the top of `public/` named like one of the three pages is a
// clash, and a refusal writes nothing, not even the directory. So the clash is
// judged before the first write, whether the entry is a file or a directory,
// and whether that directory holds anything.
//
// A file named like a page deeper in `public/` (`vendor/admin.html`) is not at
// the top of `<web-dir>`, so it clashes with nothing and is copied.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const script = path.join(here, "extract-v2-pages.mjs");
const bundle = path.resolve(here, "../tests/fixtures/pages/bundle");

const PAGES = ["admin.html", "desk.html", "receipt-sample.html"];

const run = (...args) => {
  const r = spawnSync(process.execPath, [script, ...args], { encoding: "utf8" });
  return { code: r.status, out: r.stdout, err: r.stderr };
};
/** A copy of the fixture bundle in a fresh scratch directory; `web` is beside it. */
const copyOfBundle = (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xc-extract-clash-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const copy = path.join(dir, "bundle");
  fs.cpSync(bundle, copy, { recursive: true });
  return { bundle: copy, web: path.join(dir, "web") };
};
const refused = (r, what) => {
  assert.equal(r.code, 1, what + ": exit code\n" + r.err);
  assert.equal(r.out, "", what + ": standard output");
  assert.notEqual(r.err.trim(), "", what + ": a sentence on standard error");
};

test("a directory at the top of public/ named like a page is refused before anything is written", (t) => {
  const cases = PAGES.flatMap((page) => [
    [`public/${page}/x.txt`, (dir) => {
      fs.mkdirSync(path.join(dir, "public", page));
      fs.writeFileSync(path.join(dir, "public", page, "x.txt"), "x");
    }],
    [`public/${page}/deep/y.txt`, (dir) => {
      fs.mkdirSync(path.join(dir, "public", page, "deep"), { recursive: true });
      fs.writeFileSync(path.join(dir, "public", page, "deep", "y.txt"), "y");
    }],
    [`public/${page}/ (empty)`, (dir) => fs.mkdirSync(path.join(dir, "public", page))],
  ]);
  assert.equal(cases.length, 9);
  for (const [what, clash] of cases) {
    const { bundle: dir, web } = copyOfBundle(t);
    clash(dir);
    refused(run(dir, web), what);
    assert.equal(fs.existsSync(web), false, what + ": the web directory was created");
    refused(run("--check", dir), what + " (--check)");
  }
});

test("an existing empty web directory stays empty when public/ has a directory named like a page", (t) => {
  const { bundle: dir, web } = copyOfBundle(t);
  fs.mkdirSync(path.join(dir, "public", "admin.html"));
  fs.writeFileSync(path.join(dir, "public", "admin.html", "x.txt"), "x");
  fs.mkdirSync(web);
  refused(run(dir, web), "public/admin.html/x.txt into an empty directory");
  assert.deepEqual(fs.readdirSync(web), [], "the web directory was written to");
});

test("a file named like a page deeper in public/ is copied, not refused", (t) => {
  const { bundle: dir, web } = copyOfBundle(t);
  for (const page of PAGES) fs.writeFileSync(path.join(dir, "public", "vendor", page), "<p>" + page + "</p>");
  const r = run(dir, web);
  assert.equal(r.code, 0, r.err);
  const paths = JSON.parse(r.out).files.map((f) => f.path);
  for (const page of PAGES) {
    assert.ok(paths.includes(page), page + " is in the report");
    assert.ok(paths.includes("vendor/" + page), "vendor/" + page + " is in the report");
    assert.equal(fs.readFileSync(path.join(web, "vendor", page), "utf8"), "<p>" + page + "</p>");
    assert.notEqual(fs.readFileSync(path.join(web, page), "utf8"), "<p>" + page + "</p>", page + " is the bundle's page");
  }
  const checked = run("--check", dir);
  assert.equal(checked.code, 0, checked.err);
  assert.equal(checked.out, r.out);
});
