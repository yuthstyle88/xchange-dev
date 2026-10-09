// LK-6 Look Gate test (#627, #646, #706)
// Checks:
// 1. Required screen stylesheets exist and are imported.
// 2. No .css under web/src/ declares custom properties on :root (except byte-pinned receipt fixture).
// 3. Class-name ratchet: every static class="..." in non-test screens must have a CSS rule
//    or be allowlisted in web/src/screens/look-allowlist.txt.
// Note: classList={{...}} and template-literal classes are out of scope.

import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readdirSync(dir: string): string[];
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
  statSync(file: string): { isDirectory(): boolean };
}

interface NodePath {
  resolve(...segments: string[]): string;
  join(...segments: string[]): string;
  dirname(p: string): string;
  basename(p: string, ext?: string): string;
  relative(from: string, to: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';

let fs: NodeFs;
let path: NodePath;
let WEB_ROOT: string;
let SRC_DIR: string;
let SCREENS_DIR: string;
let ALLOWLIST_PATH: string;

interface RequiredScreenCss {
  screen: string;
  cssFile: string;
  importers: string[];
}

const REQUIRED_SCREEN_STYLES: RequiredScreenCss[] = [
  {
    screen: 'devices',
    cssFile: 'screens/devices/devices.css',
    importers: ['screens/devices/DevicesScreen.tsx', 'screens/devices/main.tsx'],
  },
  {
    screen: 'kiosk-queue',
    cssFile: 'shared/ui/queue-panel.css',
    importers: ['shared/ui/QueuePanel.tsx', 'screens/kiosk-queue/KioskQueueScreen.tsx'],
  },
  {
    screen: 'kiosk-currencies',
    cssFile: 'screens/kiosk-currencies/kiosk-currencies.css',
    importers: ['screens/kiosk-currencies/KioskCurrenciesScreen.tsx', 'screens/kiosk-currencies/main.tsx'],
  },
  {
    screen: 'admin',
    cssFile: 'screens/admin/admin.css',
    importers: ['screens/admin/AdminShell.tsx', 'screens/admin/main.tsx'],
  },
  {
    screen: 'staff-guide',
    cssFile: 'screens/staff-guide/guide.css',
    importers: ['screens/staff-guide/StaffGuide.tsx', 'screens/staff-guide/main.tsx'],
  },
  {
    screen: 'receipt-sample',
    cssFile: 'screens/receipt-sample/receipt.css',
    importers: ['screens/receipt-sample/ReceiptSample.tsx', 'screens/receipt-sample/main.tsx'],
  },
  {
    screen: 'login',
    cssFile: 'shared/ui/auth.css',
    importers: ['shared/ui/AuthLayout.tsx', 'screens/login/LoginScreen.tsx'],
  },
];

function walkFiles(dir: string, filter: (name: string) => boolean): string[] {
  let results: string[] = [];
  const entries = fs.readdirSync(dir);
  for (const entry of entries) {
    const full = path.join(dir, entry);
    const stat = fs.statSync(full);
    if (stat.isDirectory()) {
      results = results.concat(walkFiles(full, filter));
    } else if (filter(entry)) {
      results.push(full);
    }
  }
  return results;
}

interface AllowlistData {
  byFolder: Map<string, Set<string>>;
  allEntries: { folder: string; name: string }[];
}

function parseAllowlist(content: string): AllowlistData {
  const byFolder = new Map<string, Set<string>>();
  const allEntries: { folder: string; name: string }[] = [];
  let currentFolder = '';

  for (const rawLine of content.split('\n')) {
    const line = rawLine.trim();
    if (!line) continue;
    if (line.startsWith('#')) {
      const match = line.match(/^#\s*([a-zA-Z0-9_\-\/]+)\s*$/);
      if (match) {
        currentFolder = match[1];
        if (!byFolder.has(currentFolder)) {
          byFolder.set(currentFolder, new Set());
        }
      }
      continue;
    }
    if (currentFolder) {
      byFolder.get(currentFolder)!.add(line);
      allEntries.push({ folder: currentFolder, name: line });
    }
  }
  return { byFolder, allEntries };
}

function getAllCssClasses(): Set<string> {
  const cssFiles = walkFiles(SRC_DIR, (f) => f.endsWith('.css'));
  const classes = new Set<string>();
  const classRegex = /\.([a-zA-Z_-][a-zA-Z0-9_-]*)/g;

  for (const file of cssFiles) {
    const content = fs.readFileSync(file, 'utf8');
    const stripped = content.replace(/\/\*[\s\S]*?\*\//g, '');
    let m: RegExpExecArray | null;
    while ((m = classRegex.exec(stripped)) !== null) {
      classes.add(m[1]);
    }
  }
  return classes;
}

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  const thisFile = new URL(import.meta.url).pathname;
  const thisDir = path.dirname(thisFile);
  WEB_ROOT = path.resolve(thisDir, '../../..');
  SRC_DIR = path.join(WEB_ROOT, 'src');
  SCREENS_DIR = path.join(SRC_DIR, 'screens');
  ALLOWLIST_PATH = path.join(SCREENS_DIR, 'look-allowlist.txt');
});

describe('screen-look gate (LK-6, #646, #706)', () => {
  it('required screen stylesheets exist and are imported', () => {
    for (const row of REQUIRED_SCREEN_STYLES) {
      const fullCss = path.join(SRC_DIR, row.cssFile);
      expect(
        fs.existsSync(fullCss),
        `Required stylesheet ${row.cssFile} for screen '${row.screen}' must exist`,
      ).toBe(true);

      let imported = false;
      const baseCss = path.basename(row.cssFile);
      for (const imp of row.importers) {
        const impPath = path.join(SRC_DIR, imp);
        if (fs.existsSync(impPath)) {
          const code = fs.readFileSync(impPath, 'utf8');
          if (new RegExp(`import\\s+['"][^'"]*${baseCss}['"]`).test(code)) {
            imported = true;
            break;
          }
        }
      }
      expect(
        imported,
        `Required stylesheet ${row.cssFile} must be imported by its screen component or entry`,
      ).toBe(true);
    }
  });

  it('no .css under web/src/ declares custom properties on :root', () => {
    const cssFiles = walkFiles(SRC_DIR, (f) => f.endsWith('.css'));
    const rootCustomPropViolations: string[] = [];

    for (const file of cssFiles) {
      const relPath = path.relative(SRC_DIR, file);
      // Byte-pinned fixture receipt.css is exempt
      if (relPath === 'screens/receipt-sample/receipt.css') {
        continue;
      }
      const content = fs.readFileSync(file, 'utf8');
      const stripped = content.replace(/\/\*[\s\S]*?\*\//g, '');
      const rootMatches = stripped.match(/:root\s*\{[^}]*--[a-zA-Z0-9_-]+\s*:/g);
      if (rootMatches) {
        rootCustomPropViolations.push(`${relPath}: ${rootMatches.join(', ')}`);
      }
    }

    expect(
      rootCustomPropViolations,
      `No stylesheet under web/src/ may declare custom properties on :root (rule 3):\n${rootCustomPropViolations.join('\n')}`,
    ).toEqual([]);
  });

  it('class-name ratchet: every static class in screens has a CSS rule or is allowlisted', () => {
    expect(
      fs.existsSync(ALLOWLIST_PATH),
      `Allowlist file ${ALLOWLIST_PATH} must exist`,
    ).toBe(true);

    const allowlistContent = fs.readFileSync(ALLOWLIST_PATH, 'utf8');
    const allowlist = parseAllowlist(allowlistContent);
    const cssClasses = getAllCssClasses();

    // Condition (b): Stale entries check - every listed name that now has a CSS rule
    const staleEntries: string[] = [];
    for (const entry of allowlist.allEntries) {
      if (cssClasses.has(entry.name)) {
        staleEntries.push(`${entry.folder}: .${entry.name}`);
      }
    }

    expect(
      staleEntries,
      `Stale look-allowlist.txt entries (these classes now have a CSS rule and must be removed from the allowlist):\n${staleEntries.join('\n')}`,
    ).toEqual([]);

    // Condition (a): Unstyled name check - static class="..." in non-test screens
    // Note: classList={{...}} and template-literal classes are out of scope.
    const prodTsxFiles = walkFiles(
      SCREENS_DIR,
      (f) => f.endsWith('.tsx') && !f.includes('.test.') && !f.includes('.spec.'),
    );
    const staticClassRegex = /\bclass=(["'])([^"'{}\n]+)\1/g;
    const unlistedUnstyled: string[] = [];

    for (const file of prodTsxFiles) {
      const rel = path.relative(SCREENS_DIR, file);
      const folder = path.dirname(rel);
      const content = fs.readFileSync(file, 'utf8');
      let m: RegExpExecArray | null;
      while ((m = staticClassRegex.exec(content)) !== null) {
        const rawClassValue = m[2];
        const tokens = rawClassValue.split(/\s+/).map((t) => t.trim()).filter(Boolean);
        for (const cls of tokens) {
          if (cssClasses.has(cls)) {
            continue;
          }
          const folderAllowlist = allowlist.byFolder.get(folder);
          if (!folderAllowlist || !folderAllowlist.has(cls)) {
            unlistedUnstyled.push(`${folder} (${path.basename(file)}): .${cls}`);
          }
        }
      }
    }

    expect(
      unlistedUnstyled,
      `New unstyled class names found in screens (must have a CSS rule or be listed in look-allowlist.txt):\n${unlistedUnstyled.join('\n')}`,
    ).toEqual([]);
  });
});
