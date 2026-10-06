// Records how the v2 server judges an uploaded file, for `api/tests/files_test.rs`.
//
// Needs `upload-harness.mjs` beside it — lines 7126-7151 of the deployed
// bundle's `server.mjs` (`xchange-online@3ffc5a5`, the function
// `uploadEvidence`), unchanged, behind these lines:
//
//   import { createHash as createHash3, randomUUID as randomUUID5 } from "node:crypto";
//   export const calls = { execute: [], put: [], record: [] };
//   const execute = async (action, payload, actor) => { calls.execute.push([action, payload]); };
//   const env = { BUCKET: { put: async (key, bytes, options) => { calls.put.push([key, Buffer.from(bytes), options]); } } };
//   const recordUpload = async (key, branchId, actor, mime, hash) => { calls.record.push([key, branchId, actor.id, mime, hash]); };
//   <lines 7126-7151>
//   export { uploadEvidence };
//
// The bundle is not in this repository, so this script cannot run in CI; its
// output is checked in and is what the tests read.
//
//   node gen_uploads.mjs > ../../tests/fixtures/files/v2_upload_cases.json
//
// A case's body is `head` (hex) followed by `pad` zero bytes, so a 15 MB body
// costs one line.

import { uploadEvidence, calls } from "./upload-harness.mjs";

const hex = (s) => Buffer.from(s, "latin1").toString("hex");
const JPEG = "ffd8ffe000104a464946", PNG = "89504e470d0a1a0a0000000d", PDF = hex("%PDF-1.7\n"), WEBP = hex("RIFF") + "24000000" + hex("WEBPVP8 ");
const LIMIT = 15 * 1024 * 1024;

const cases = [];
async function add(label, mime, head, pad = 0) {
  const bytes = Buffer.concat([Buffer.from(head, "hex"), Buffer.alloc(pad)]);
  calls.execute.length = calls.put.length = calls.record.length = 0;
  const headers = {};
  if (mime !== null) headers["content-type"] = mime;
  const request = new Request("http://xc.test/api/upload", { method: "POST", headers, body: bytes, duplex: "half" });
  const out = { label, mime, head, pad, size: bytes.length };
  try {
    const result = await uploadEvidence(request, { id: "owner" }, "MAIN");
    const [key, stored, options] = calls.put[0];
    const [rkey, branch, owner, rmime, hash] = calls.record[0];
    if (key !== result.uploadKey || rkey !== key || !stored.equals(bytes) || options.httpMetadata.contentType !== mime || rmime !== mime) throw Error("harness: v2 stored something else ");
    if (!/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/.test(key)) throw Error("harness: key shape " + key);
    out.accepted = true;
    out.sha256 = hash;
  } catch (e) {
    if (String(e.message).startsWith("harness:")) throw e;
    out.accepted = false;
    out.error = e.message;
    if (calls.put.length || calls.record.length) throw Error("harness: v2 stored a refused file");
  }
  // the branch is looked at first, whatever the file is
  if (calls.execute.length !== 1 || calls.execute[0][0] !== "adminDeskConfig" || calls.execute[0][1].branchId !== "MAIN") throw Error("harness: branch check");
  cases.push(out);
}

// the four kinds, smallest and ordinary
await add("a JPEG", "image/jpeg", JPEG, 200);
await add("a JPEG of two bytes", "image/jpeg", "ffd8");
await add("a PNG", "image/png", PNG, 200);
await add("a PNG of exactly its signature", "image/png", "89504e470d0a1a0a");
await add("a PDF", "application/pdf", PDF, 200);
await add("a PDF of exactly five bytes", "application/pdf", hex("%PDF-"));
await add("a WebP", "image/webp", WEBP, 200);
await add("a WebP of exactly twelve bytes", "image/webp", hex("RIFF") + "00000000" + hex("WEBP"));
await add("a WebP whose four length bytes are anything", "image/webp", hex("RIFF") + "ffffffff" + hex("WEBP"), 8);
// content that is not what the type says
await add("a JPEG of one byte", "image/jpeg", "ff");
await add("a JPEG whose second byte is wrong", "image/jpeg", "ffd9ffe0", 20);
await add("a JPEG whose first byte is wrong", "image/jpeg", "fed8ffe0", 20);
await add("a PNG sent as JPEG", "image/jpeg", PNG, 20);
await add("a JPEG sent as PNG", "image/png", JPEG, 20);
await add("a PNG one byte short of its signature", "image/png", "89504e470d0a1a");
await add("a PNG whose last signature byte is wrong", "image/png", "89504e470d0a1a0b", 20);
await add("a PDF in lower case", "application/pdf", hex("%pdf-1.7"), 20);
await add("a PDF without the dash", "application/pdf", hex("%PDF1.7"), 20);
await add("a PDF after a blank", "application/pdf", hex(" %PDF-1.7"), 20);
await add("a PDF of four bytes", "application/pdf", hex("%PDF"));
await add("a WebP of eleven bytes", "image/webp", hex("RIFF") + "00000000" + hex("WEB"));
await add("a WAVE file sent as WebP", "image/webp", hex("RIFF") + "24000000" + hex("WAVEfmt "), 20);
await add("a WebP without RIFF", "image/webp", hex("RIFX") + "24000000" + hex("WEBPVP8 "), 20);
await add("a PDF sent as WebP", "image/webp", PDF, 20);
await add("a JPEG sent as PDF", "application/pdf", JPEG, 20);
await add("an HTML page sent as JPEG", "image/jpeg", hex("<!doctype html><script>"), 0);
// nothing at all
for (const mime of ["image/jpeg", "image/png", "image/webp", "application/pdf"]) await add("an empty body", mime, "");
// kinds that are not taken, whatever the content
await add("no type at all", null, JPEG, 20);
await add("an empty type", "", JPEG, 20);
await add("a type with a parameter", "image/jpeg; charset=binary", JPEG, 20);
await add("a type in upper case", "IMAGE/JPEG", JPEG, 20);
// (a blank around the type never reaches this code: the HTTP layer trims header values)
await add("image/jpg is not image/jpeg", "image/jpg", JPEG, 20);
await add("a GIF", "image/gif", hex("GIF89a"), 20);
await add("an SVG", "image/svg+xml", hex("<svg/>"), 0);
await add("an HTML page", "text/html", hex("<!doctype html>"), 0);
await add("a HEIC photo", "image/heic", "000000186674797068656963", 20);
await add("plain bytes", "application/octet-stream", JPEG, 20);
await add("a form upload", "multipart/form-data; boundary=x", JPEG, 20);
await add("an unsupported type with an empty body", "text/plain", "");
// the size limit: 15 MB = 15 * 1024 * 1024 bytes
await add("exactly 15 MB", "image/jpeg", JPEG, LIMIT - JPEG.length / 2);
await add("one byte over 15 MB", "image/jpeg", JPEG, LIMIT - JPEG.length / 2 + 1);
await add("one byte under 15 MB", "application/pdf", PDF, LIMIT - PDF.length / 2 - 1);
await add("one byte over 15 MB whose content is wrong too", "image/png", JPEG, LIMIT - JPEG.length / 2 + 1);
await add("one byte over 15 MB of an unsupported type", "image/gif", hex("GIF89a"), LIMIT);

const sentences = [...new Set(cases.filter((c) => !c.accepted).map((c) => c.error))].sort();
process.stdout.write(JSON.stringify({ source: "xchange-online@3ffc5a5 server.mjs:7126-7151", limit: LIMIT, sentences, cases }, null, 1) + "\n");
