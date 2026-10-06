#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";

function refuse(message) {
  console.error(message);
  process.exit(1);
}

function sha(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function parseArguments() {
  const args = process.argv.slice(2);
  if (args.length === 2 && args[0] === "--check") {
    return { checkMode: true, bundleDir: path.resolve(args[1]), webDir: null };
  }
  if (args.length === 2 && args[0] !== "--check" && args[1] !== "--check") {
    return {
      checkMode: false,
      bundleDir: path.resolve(args[0]),
      webDir: path.resolve(args[1]),
    };
  }
  refuse("Usage: node extract-v2-pages.mjs <bundle-dir> <web-dir> OR --check <bundle-dir>");
}

function scanPublic(publicDir, relBase = "") {
  const dir = path.join(publicDir, relBase);
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch (err) {
    refuse(`Cannot read directory ${dir}: ${err.message}`);
  }

  const result = [];
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    const relPath = relBase ? relBase + "/" + entry.name : entry.name;
    let stat;
    try {
      stat = fs.lstatSync(fullPath);
    } catch (err) {
      refuse(`Cannot stat ${fullPath}: ${err.message}`);
    }

    if (stat.isSymbolicLink() || (!stat.isFile() && !stat.isDirectory())) {
      refuse(`Non-regular file or symlink found in public: ${relPath}`);
    }

    if (stat.isDirectory()) {
      result.push(...scanPublic(publicDir, relPath));
    } else if (stat.isFile()) {
      let bytes;
      try {
        bytes = fs.readFileSync(fullPath);
      } catch (err) {
        refuse(`Cannot read file ${fullPath}: ${err.message}`);
      }
      result.push({ relPath, fullPath, bytes });
    }
  }
  return result;
}

function extractTemplateLiteral(serverText, name) {
  // Find declarations at line start: (^|\n)var <name> = ...
  const prefix = `var ${name} = `;
  let searchPos = 0;
  const matchIndices = [];

  while (true) {
    const idx = serverText.indexOf(prefix, searchPos);
    if (idx === -1) break;
    if (idx === 0 || serverText[idx - 1] === "\n") {
      matchIndices.push(idx);
    }
    searchPos = idx + prefix.length;
  }

  if (matchIndices.length === 0) {
    refuse(`Declaration for ${name} is missing`);
  }
  if (matchIndices.length > 1) {
    refuse(`Declaration for ${name} is declared twice`);
  }

  const declStart = matchIndices[0];
  const afterPrefix = serverText.slice(declStart + prefix.length);
  if (!afterPrefix.startsWith("`")) {
    refuse(`Declaration for ${name} is not a template literal`);
  }

  const startContentIndex = declStart + prefix.length + 1;
  let inEscape = false;
  let closed = false;
  let endIndex = -1;

  for (let i = startContentIndex; i < serverText.length; i++) {
    const ch = serverText[i];
    if (inEscape) {
      inEscape = false;
      continue;
    }
    if (ch === "\\") {
      inEscape = true;
      continue;
    }
    if (ch === "$" && i + 1 < serverText.length && serverText[i + 1] === "{") {
      refuse(`Template literal for ${name} has an interpolation`);
    }
    if (ch === "`") {
      closed = true;
      endIndex = i;
      break;
    }
  }

  if (!closed) {
    refuse(`Template literal for ${name} never ends`);
  }

  const afterBacktick = serverText.slice(endIndex + 1);
  if (!afterBacktick.startsWith(";")) {
    refuse(`Declaration for ${name} is followed by more than the semicolon`);
  }

  const afterSemi = afterBacktick.slice(1);
  const nextLineIdx = afterSemi.search(/\r?\n|$/);
  const onSameLine = afterSemi.slice(0, nextLineIdx).trim();
  if (onSameLine !== "") {
    refuse(`Declaration for ${name} is followed by more than the semicolon`);
  }

  const literalContent = serverText.slice(startContentIndex, endIndex);
  let pageContent;
  try {
    const fn = new Function("return `" + literalContent + "`;");
    pageContent = fn();
  } catch (err) {
    refuse(`Failed to evaluate template literal for ${name}: ${err.message}`);
  }

  return Buffer.from(pageContent, "utf8");
}

function main() {
  const { checkMode, bundleDir, webDir } = parseArguments();

  // Validate webDir if not checkMode
  if (!checkMode) {
    if (fs.existsSync(webDir)) {
      let stat;
      try {
        stat = fs.statSync(webDir);
      } catch (err) {
        refuse(`Cannot stat web directory ${webDir}: ${err.message}`);
      }
      if (!stat.isDirectory()) {
        refuse(`Target path ${webDir} exists and is not a directory`);
      }
      let entries;
      try {
        entries = fs.readdirSync(webDir);
      } catch (err) {
        refuse(`Cannot read target directory ${webDir}: ${err.message}`);
      }
      if (entries.length > 0) {
        refuse(`Target directory ${webDir} is not empty`);
      }
    }
  }

  // Validate bundleDir
  if (!fs.existsSync(bundleDir)) {
    refuse(`Bundle directory ${bundleDir} does not exist`);
  }
  let bundleStat;
  try {
    bundleStat = fs.statSync(bundleDir);
  } catch (err) {
    refuse(`Cannot stat bundle directory ${bundleDir}: ${err.message}`);
  }
  if (!bundleStat.isDirectory()) {
    refuse(`Bundle path ${bundleDir} is not a directory`);
  }

  const serverFile = path.join(bundleDir, "server.mjs");
  if (!fs.existsSync(serverFile) || !fs.statSync(serverFile).isFile()) {
    refuse(`server.mjs missing or not a file in ${bundleDir}`);
  }

  const publicDir = path.join(bundleDir, "public");
  if (!fs.existsSync(publicDir) || !fs.statSync(publicDir).isDirectory()) {
    refuse(`public/ missing or not a directory in ${bundleDir}`);
  }

  // Read and validate server.mjs
  let serverText;
  try {
    serverText = fs.readFileSync(serverFile, "utf8");
  } catch (err) {
    refuse(`Cannot read ${serverFile}: ${err.message}`);
  }

  const adminBuffer = extractTemplateLiteral(serverText, "admin_html_default");
  const deskBuffer = extractTemplateLiteral(serverText, "index_html_default");
  const receiptBuffer = extractTemplateLiteral(serverText, "receipt_html_default");

  // Read public files
  const publicFiles = scanPublic(publicDir);

  const pageNames = new Set(["admin.html", "desk.html", "receipt-sample.html"]);
  for (const pf of publicFiles) {
    if (pageNames.has(pf.relPath)) {
      refuse(`public/ already has ${pf.relPath}`);
    }
  }

  // Build report items
  const allFiles = [
    {
      path: "admin.html",
      bytes: adminBuffer.length,
      sha256: sha(adminBuffer),
      buffer: adminBuffer,
    },
    {
      path: "desk.html",
      bytes: deskBuffer.length,
      sha256: sha(deskBuffer),
      buffer: deskBuffer,
    },
    {
      path: "receipt-sample.html",
      bytes: receiptBuffer.length,
      sha256: sha(receiptBuffer),
      buffer: receiptBuffer,
    },
    ...publicFiles.map((pf) => ({
      path: pf.relPath,
      bytes: pf.bytes.length,
      sha256: sha(pf.bytes),
      buffer: pf.bytes,
    })),
  ];

  allFiles.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));

  // Write if not checkMode
  if (!checkMode) {
    try {
      fs.mkdirSync(webDir, { recursive: true });
    } catch (err) {
      refuse(`Failed to create directory ${webDir}: ${err.message}`);
    }

    for (const file of allFiles) {
      const targetPath = path.join(webDir, file.path);
      try {
        fs.mkdirSync(path.dirname(targetPath), { recursive: true });
        fs.writeFileSync(targetPath, file.buffer);
      } catch (err) {
        refuse(`Failed to write file ${targetPath}: ${err.message}`);
      }
    }
  }

  const report = {
    count: allFiles.length,
    files: allFiles.map((f) => ({
      bytes: f.bytes,
      path: f.path,
      sha256: f.sha256,
    })),
  };

  process.stdout.write(JSON.stringify(report) + "\n");
  process.exit(0);
}

main();
