// Specification tests for `extract-v2-pages.mjs` (ticket T1c).
//
// Written before the implementation. Do not edit, delete or weaken.
//
//   node --test api/tools/extract-v2-pages.test.mjs        (Node 20 or newer)
//
// The script writes the directory the server's `web.dir` setting names, from a
// v2 bundle on the machine it is deployed to:
//
//   node extract-v2-pages.mjs <bundle-dir> <web-dir>     write <web-dir>
//   node extract-v2-pages.mjs --check <bundle-dir>       write nothing
//
// Both print one JSON document on standard output and exit 0:
//   { "count": N, "files": [ { "path", "bytes", "sha256" }, ... ] }   sorted by path
// Any refusal: exit code 1, a sentence on standard error, nothing on standard
// output, and nothing written — not even the directory.
//
// `<bundle-dir>/server.mjs` holds three pages as template literals:
//   var admin_html_default   = `...`;   ->  admin.html
//   var index_html_default   = `...`;   ->  desk.html
//   var receipt_html_default = `...`;   ->  receipt-sample.html
// and `<bundle-dir>/public/` is copied as it is.
//
// The bundle used here, `api/tests/fixtures/pages/bundle/`, is a stand-in
// written for these tests. It is a module that can be imported, so what the
// script writes is compared with what the bundle's own code hands out.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const script = path.join(here, "extract-v2-pages.mjs");
const bundle = path.resolve(here, "../tests/fixtures/pages/bundle");

const run = (...args) => {
  const r = spawnSync(process.execPath, [script, ...args], { encoding: "utf8" });
  return { code: r.status, out: r.stdout, err: r.stderr };
};
const scratch = (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xc-extract-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
};
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
/** Every file under `dir`, as { "relative/path": Buffer }, `/` between parts. */
const tree = (dir, base = "") => {
  const out = {};
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const rel = base ? base + "/" + e.name : e.name;
    if (e.isDirectory()) Object.assign(out, tree(path.join(dir, e.name), rel));
    else out[rel] = fs.readFileSync(path.join(dir, e.name));
  }
  return out;
};
/** A copy of the fixture bundle the test may change. */
const copyOfBundle = (t) => {
  const dir = path.join(scratch(t), "bundle");
  fs.cpSync(bundle, dir, { recursive: true });
  return dir;
};
const editServer = (dir, change) => {
  const file = path.join(dir, "server.mjs");
  fs.writeFileSync(file, change(fs.readFileSync(file, "utf8")));
};
const refused = (r, what) => {
  assert.equal(r.code, 1, what + ": exit code\n" + r.err);
  assert.equal(r.out, "", what + ": standard output");
  assert.notEqual(r.err.trim(), "", what + ": a sentence on standard error");
};

const PAGES = ["admin.html", "desk.html", "receipt-sample.html"];
const PUBLIC = [".keep-me", "app.js", "fonts/all-bytes.woff2", "login.html", "style.css", "vendor/LICENSE.txt", "vendor/deep/empty.txt"];
const ALL = [...PAGES, ...PUBLIC].sort();

test("the three pages come out as the bundle's own code hands them out", async (t) => {
  const web = path.join(scratch(t), "web");
  const r = run(bundle, web);
  assert.equal(r.code, 0, r.err);

  const m = await import(pathToFileURL(path.join(bundle, "server.mjs")).href);
  const request = {};
  assert.deepEqual(fs.readFileSync(path.join(web, "admin.html")), Buffer.from(m.GET(request), "utf8"));
  assert.deepEqual(fs.readFileSync(path.join(web, "desk.html")), Buffer.from(m.GET2(request), "utf8"));
  assert.deepEqual(fs.readFileSync(path.join(web, "receipt-sample.html")), Buffer.from(m.GET3(request), "utf8"));

  // and spelled out, so a broken oracle cannot hide a broken script
  const admin = fs.readFileSync(path.join(web, "admin.html"), "utf8");
  assert.ok(admin.startsWith('<!doctype html>\n<html lang="th">\n<head><title>XCHANGE · แอดมิน (bundle fixture)</title></head>'), admin);
  assert.ok(admin.includes('const tick = "`", dollar = "${not.interpolated}", slash = "a\\\\b", nl = "x\\ny", u = "ก\u{1f600}";'), admin);
  assert.ok(admin.endsWith("</body>\n</html>\n"), "the newline before the closing backtick is part of the page");
  assert.equal(
    fs.readFileSync(path.join(web, "receipt-sample.html"), "utf8"),
    '<!doctype html>\n<html lang="en"><head><title>ticket • sample (bundle fixture)</title></head><body>&amp; $1</body></html>',
  );
});

test("public/ is copied byte for byte, and nothing else is written", (t) => {
  const web = path.join(scratch(t), "web");
  const r = run(bundle, web);
  assert.equal(r.code, 0, r.err);

  const written = tree(web);
  assert.deepEqual(Object.keys(written).sort(), ALL);
  const source = tree(path.join(bundle, "public"));
  for (const rel of PUBLIC) assert.deepEqual(written[rel], source[rel], rel);
  assert.equal(written["fonts/all-bytes.woff2"].length, 256);
  assert.equal(written["vendor/deep/empty.txt"].length, 0);
  // the bundle is only read
  assert.deepEqual(Object.keys(tree(bundle)).sort(), ["server.mjs", ...PUBLIC.map((p) => "public/" + p)].sort());
});

test("the report lists every file with its size and SHA-256, sorted by path", (t) => {
  const web = path.join(scratch(t), "web");
  const r = run(bundle, web);
  assert.equal(r.code, 0, r.err);
  const report = JSON.parse(r.out);
  assert.equal(report.count, ALL.length);
  assert.deepEqual(report.files.map((f) => f.path), ALL);
  const written = tree(web);
  for (const f of report.files) {
    assert.deepEqual(Object.keys(f).sort(), ["bytes", "path", "sha256"]);
    assert.equal(f.bytes, written[f.path].length, f.path);
    assert.equal(f.sha256, sha(written[f.path]), f.path);
  }
});

test("--check prints the same report and writes nothing", (t) => {
  const dir = scratch(t);
  const written = run(bundle, path.join(dir, "web"));
  const before = fs.readdirSync(dir);
  const checked = run("--check", bundle);
  assert.equal(checked.code, 0, checked.err);
  assert.equal(checked.out, written.out);
  assert.deepEqual(fs.readdirSync(dir), before);
});

test("an existing empty directory is filled; one with anything in it is refused untouched", (t) => {
  const dir = scratch(t);
  const empty = path.join(dir, "empty");
  fs.mkdirSync(empty);
  assert.equal(run(bundle, empty).code, 0);
  assert.deepEqual(Object.keys(tree(empty)).sort(), ALL);

  const used = path.join(dir, "used");
  fs.mkdirSync(used);
  fs.writeFileSync(path.join(used, "old.txt"), "old");
  refused(run(bundle, used), "a directory with a file in it");
  assert.deepEqual(Object.keys(tree(used)), ["old.txt"]);
  // running twice into the same place is that case
  refused(run(bundle, empty), "a second run into the same directory");

  const file = path.join(dir, "a-file");
  fs.writeFileSync(file, "x");
  refused(run(bundle, file), "a file where the directory should be");
  assert.equal(fs.readFileSync(file, "utf8"), "x");
});

test("a bundle that is not the expected shape is refused and nothing is written", (t) => {
  const broken = [
    ["no server.mjs", (dir) => fs.rmSync(path.join(dir, "server.mjs"))],
    ["no public/", (dir) => fs.rmSync(path.join(dir, "public"), { recursive: true })],
    ["public is a file", (dir) => { fs.rmSync(path.join(dir, "public"), { recursive: true }); fs.writeFileSync(path.join(dir, "public"), ""); }],
    ...["admin_html_default", "index_html_default", "receipt_html_default"].flatMap((name) => [
      [`${name} is missing`, (dir) => editServer(dir, (s) => s.replace(`\nvar ${name} = `, `\nvar ${name}_gone = `))],
      [`${name} is declared twice`, (dir) => editServer(dir, (s) => s + `\nvar ${name} = \`<p>again</p>\`;\n`)],
      [`${name} is not a template literal`, (dir) => editServer(dir, (s) => s.replace(`\nvar ${name} = \``, `\nvar ${name} = "x" + \``))],
      [`${name} has an interpolation`, (dir) => editServer(dir, (s) => s.replace(`var ${name} = \`<!doctype html>`, `var ${name} = \`<!doctype html>\${1 + 1}`))],
      [`${name} is followed by more than the semicolon`, (dir) => editServer(dir, (s) => {
        const at = s.indexOf("`;", s.indexOf(`\nvar ${name} = \``));
        return s.slice(0, at) + "` + decoy;" + s.slice(at + 2);
      })],
    ]),
    ["the last page never ends", (dir) => editServer(dir, (s) => s.slice(0, s.indexOf("</body></html>`;\nfunction GET3")))],
    ...PAGES.map((file) => [`public/ already has ${file}`, (dir) => fs.writeFileSync(path.join(dir, "public", file), "<p>clash</p>")]),
  ];
  assert.equal(broken.length, 22);
  for (const [what, breakIt] of broken) {
    const dir = copyOfBundle(t);
    breakIt(dir);
    const web = path.join(path.dirname(dir), "web");
    refused(run(dir, web), what);
    assert.equal(fs.existsSync(web), false, what + ": the web directory was created");
    refused(run("--check", dir), what + " (--check)");
  }
});

test("a declaration inside a string, or indented, is not a page", (t) => {
  // the fixture has `var decoy = "var admin_html_default = \`...\`;"` — already
  // covered by the first test. An indented declaration is somebody's local.
  const dir = copyOfBundle(t);
  editServer(dir, (s) => s + "\nfunction f() {\n  var admin_html_default = `<p>local</p>`;\n  return admin_html_default;\n}\n");
  const web = path.join(path.dirname(dir), "web");
  const r = run(dir, web);
  assert.equal(r.code, 0, r.err);
  assert.ok(fs.readFileSync(path.join(web, "admin.html"), "utf8").includes("(bundle fixture)"));
});

test("a link or anything else that is not a file or a directory under public/ is refused", { skip: process.platform === "win32" }, (t) => {
  const dir = copyOfBundle(t);
  fs.symlinkSync(path.join(dir, "server.mjs"), path.join(dir, "public", "link.js"));
  const web = path.join(path.dirname(dir), "web");
  refused(run(dir, web), "a link in public/");
  assert.equal(fs.existsSync(web), false);
});

test("the arguments are two directories, or --check and one", (t) => {
  const dir = scratch(t);
  for (const args of [[], [bundle], ["--check"], ["--check", bundle, path.join(dir, "w")], [bundle, path.join(dir, "w"), "extra"]]) {
    refused(run(...args), JSON.stringify(args));
  }
  assert.equal(fs.existsSync(path.join(dir, "w")), false);
});
