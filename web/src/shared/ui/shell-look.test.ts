import { beforeAll, describe, expect, it } from 'vitest';
import { createComponent } from 'solid-js';
import { render } from 'solid-js/web';
import { AppShell } from './AppShell';

interface NodeFs {
  readdirSync(dir: string): string[];
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
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
let PAGES_DIR: string;
let APP_SHELL_PATH: string;

// Explicit list of entry HTML files whose <script src> module reaches AppShell or AuthLayout
// through one or more child modules rather than directly importing them in main.tsx.
const CHILD_MODULE_ENTRIES: Record<string, string[]> = {
  'admin.html': ['/src/screens/admin/AdminShell.tsx'],
  'devices.html': ['/src/screens/devices/DevicesScreen.tsx'],
  'login.html': ['/src/screens/login/LoginScreen.tsx'],
  'receipt-sample.html': ['/src/screens/receipt-sample/ReceiptSample.tsx'],
  'staff-guide.html': ['/src/screens/staff-guide/StaffGuide.tsx'],
  'staff-guide-html.html': ['/src/screens/staff-guide/StaffGuide.tsx'],
  'staff.html': ['/src/screens/staff/StaffPage.tsx'],
};

// Public pages that deliberately have no shell and read no session, with the reason. LK-1 skips only these; the
// guard below keeps every one of them an `anyone` page, so the list can never hide a staff page.
// (RD-3, xchange FE lane controller claude/5edf7bb5's ruling A, 2026-10-11.)
const SHELL_LESS_PUBLIC_PAGES: Record<string, string> = {
  'rates.html': 'P01 public board: no shell, no session (rate-display.md D3, D8)',
};

function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/\/\/[^\r\n]*/g, '');
}

function hasShellOrAuthLayoutImport(source: string): boolean {
  const clean = stripComments(source);
  return (
    /(?:^|\n)\s*import\s+[^;]*from\s+['"][^'"]*(?:AppShell|AuthLayout)['"]/.test(clean) ||
    /(?:^|\n)\s*import\s+['"][^'"]*(?:AppShell|AuthLayout)['"]/.test(clean)
  );
}

function rendersShellOrAuthLayout(source: string): boolean {
  const clean = stripComments(source);
  return /<\s*(AppShell|AuthLayout)\b/.test(clean);
}

function hasCustomerShellImport(source: string): boolean {
  const clean = stripComments(source);
  return /(?:^|\n)\s*import\s+\{[^}]*\bCustomerShell\b[^}]*\}\s*from\s+['"][^'"]*(?:screens\/customer\/)?_shell\/Shell(?:\.tsx)?['"]/.test(clean);
}

function rendersCustomerShell(source: string): boolean {
  const clean = stripComments(source);
  return /<\s*CustomerShell\b/.test(clean);
}


beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  const thisFile = new URL(import.meta.url).pathname;
  const thisDir = path.dirname(thisFile);
  WEB_ROOT = path.resolve(thisDir, '../../..');
  PAGES_DIR = path.join(WEB_ROOT, 'pages');
  APP_SHELL_PATH = path.join(thisDir, 'AppShell.tsx');
});

describe('shell-look gate (LK-1, #627, #630)', () => {
  it('AppShell.tsx imports ./shell.css', () => {
    const content = fs.readFileSync(APP_SHELL_PATH, 'utf-8');
    expect(content).toMatch(/(?:^|\n)\s*import\s+['"]\.\/shell\.css['"]/);
  });

  it('every web/pages/*.html entry reaches AppShell or AuthLayout', () => {
    const htmlFiles = fs.readdirSync(PAGES_DIR).filter((f: string) => f.endsWith('.html'));
    expect(htmlFiles.length).toBeGreaterThan(0);

    for (const file of htmlFiles) {
      const htmlPath = path.join(PAGES_DIR, file);
      const htmlContent = fs.readFileSync(htmlPath, 'utf-8');

      // Extract script src
      const scriptMatch = htmlContent.match(/<script\s+[^>]*src=["']([^"']+)["']/i);
      expect(scriptMatch, `${file} must contain a <script src="..."> tag`).not.toBeNull();

      const scriptSrc = scriptMatch![1];
      // Resolve path relative to web root (e.g. /src/screens/... -> web/src/screens/...)
      const scriptRelative = scriptSrc.replace(/^\//, '');
      const scriptFullPath = path.join(WEB_ROOT, scriptRelative);

      expect(fs.existsSync(scriptFullPath), `${scriptFullPath} exists`).toBe(true);
      const scriptSource = stripComments(fs.readFileSync(scriptFullPath, 'utf-8'));

      if (Object.prototype.hasOwnProperty.call(SHELL_LESS_PUBLIC_PAGES, file)) {
        // A named shell-less public page; the guard test below pins it to `anyone`.
        continue;
      }

      if (file.startsWith('app-')) {
        // Customer screen entry reaches AppShell via CustomerShell (#998)
        if (hasCustomerShellImport(scriptSource) && rendersCustomerShell(scriptSource)) {
          continue;
        }
        expect(
          hasCustomerShellImport(scriptSource),
          `Customer entry ${file} (${scriptRelative}) must import CustomerShell from .../screens/customer/_shell/Shell`,
        ).toBe(true);
        expect(
          rendersCustomerShell(scriptSource),
          `Customer entry ${file} (${scriptRelative}) must render <CustomerShell`,
        ).toBe(true);
      }

      if (hasShellOrAuthLayoutImport(scriptSource) && rendersShellOrAuthLayout(scriptSource)) {
        // Direct import and render in entry module
        continue;
      }

      // Check explicit child module listing
      const childModules = CHILD_MODULE_ENTRIES[file];
      expect(
        childModules && childModules.length > 0,
        `${file} does not directly render AppShell or AuthLayout, so it must be explicitly listed in CHILD_MODULE_ENTRIES`,
      ).toBe(true);

      const entryDir = path.dirname(scriptFullPath);

      for (const childRel of childModules!) {
        const childFullPath = path.join(WEB_ROOT, childRel.replace(/^\//, ''));
        expect(fs.existsSync(childFullPath), `Child module ${childFullPath} must exist`).toBe(true);

        // Assert that the entry module imports this child module
        let relSpecifier = path.relative(entryDir, childFullPath).replace(/\.tsx?$/, '');
        if (!relSpecifier.startsWith('.')) {
          relSpecifier = './' + relSpecifier;
        }
        const escapedRel = relSpecifier.replace(/[-/\\^$*+?.()|[\]{}]/g, '\\$&');
        const importPattern = new RegExp(`(?:^|\\n)\\s*import\\s+[^;]*from\\s+['"]${escapedRel}['"]`);

        expect(
          importPattern.test(scriptSource),
          `Entry module ${scriptRelative} for ${file} must import child module ${childRel} (${relSpecifier})`,
        ).toBe(true);

        // Assert that the child module imports and renders AppShell or AuthLayout
        const childSource = stripComments(fs.readFileSync(childFullPath, 'utf-8'));
        expect(
          hasShellOrAuthLayoutImport(childSource),
          `Child module ${childRel} reached from ${file} must import AppShell or AuthLayout`,
        ).toBe(true);
        expect(
          rendersShellOrAuthLayout(childSource),
          `Child module ${childRel} reached from ${file} must render AppShell or AuthLayout`,
        ).toBe(true);
      }
    }

    // Customer app shell composition: Shell.tsx must exist and compose AppShell
    const customerShellPath = path.join(WEB_ROOT, 'src/screens/customer/_shell/Shell.tsx');
    expect(fs.existsSync(customerShellPath), 'web/src/screens/customer/_shell/Shell.tsx must exist').toBe(true);
    const customerShellSource = stripComments(fs.readFileSync(customerShellPath, 'utf-8'));
    expect(
      hasShellOrAuthLayoutImport(customerShellSource),
      'web/src/screens/customer/_shell/Shell.tsx must import AppShell',
    ).toBe(true);
    expect(
      rendersShellOrAuthLayout(customerShellSource),
      'web/src/screens/customer/_shell/Shell.tsx must render <AppShell',
    ).toBe(true);
  });

  it('every shell-less public page exists in web/pages and is an anyone page', () => {
    const entries = Object.keys(SHELL_LESS_PUBLIC_PAGES);
    expect(entries.length).toBeGreaterThan(0);
    for (const file of entries) {
      expect(SHELL_LESS_PUBLIC_PAGES[file]!.trim(), `${file}: a reason`).not.toBe('');
      const htmlPath = path.join(PAGES_DIR, file);
      expect(fs.existsSync(htmlPath), `${file} exists in web/pages`).toBe(true);
      const htmlContent = fs.readFileSync(htmlPath, 'utf-8');
      const gates = [...htmlContent.matchAll(/<meta\s+name=["']xc-gate["']\s+content=["']([^"']*)["']/gi)].map((m) => m[1]);
      expect(gates, `${file}: exactly one xc-gate, and it is anyone`).toEqual(['anyone']);
    }
  });

  it('detector: a bare import plus a comment fails the shell gate', () => {
    // Fake customer shell: bare import plus JSX in comment
    const fakeCustomer = `
      import '../_shell/Shell';
      // <CustomerShell />
    `;
    expect(
      hasCustomerShellImport(fakeCustomer) && rendersCustomerShell(fakeCustomer),
      'bare import plus comment must fail customer rule',
    ).toBe(false);
    expect(hasCustomerShellImport(fakeCustomer), 'bare CustomerShell import is rejected').toBe(false);
    expect(rendersCustomerShell(fakeCustomer), 'commented <CustomerShell /> is not rendered').toBe(false);

    // Fake AppShell: bare import plus JSX in comment
    const fakeAppShell = `
      import '../../../shared/ui/AppShell';
      // <AppShell />
    `;
    expect(
      hasShellOrAuthLayoutImport(fakeAppShell) && rendersShellOrAuthLayout(fakeAppShell),
      'bare import plus comment must fail AppShell rule',
    ).toBe(false);
    expect(rendersShellOrAuthLayout(fakeAppShell), 'commented <AppShell /> is not rendered').toBe(false);

    // Also verify block comments
    const fakeBlockCustomer = `
      import '../_shell/Shell';
      /* <CustomerShell /> */
    `;
    expect(
      hasCustomerShellImport(fakeBlockCustomer) && rendersCustomerShell(fakeBlockCustomer),
      'bare import plus block comment must fail customer rule',
    ).toBe(false);

    const fakeBlockAppShell = `
      import '../../../shared/ui/AppShell';
      /* <AppShell /> */
    `;
    expect(
      hasShellOrAuthLayoutImport(fakeBlockAppShell) && rendersShellOrAuthLayout(fakeBlockAppShell),
      'bare import plus block comment must fail AppShell rule',
    ).toBe(false);
  });

  describe('AppShell look prop rendering (second revision)', () => {
    it('renders <div class="xc-shell"> when look is omitted', () => {
      const host = document.createElement('div');
      const dispose = render(() => createComponent(AppShell, { title: 'Default Shell' }), host);
      const root = host.firstElementChild;
      expect(root).not.toBeNull();
      expect(root?.className).toBe('xc-shell');
      expect(root?.classList.contains('xc-shell')).toBe(true);
      expect(root?.classList.contains('xc-devices')).toBe(false);
      expect(root?.classList.contains('xc-kiosk')).toBe(false);
      expect(root?.classList.contains('xc-admin')).toBe(false);
      expect(host.innerHTML).not.toContain('xc-undefined');
      dispose();
    });

    it('renders <div class="xc-shell xc-devices"> when look="devices"', () => {
      const host = document.createElement('div');
      const dispose = render(
        () => createComponent(AppShell, { title: 'Devices', look: 'devices' }),
        host,
      );
      const root = host.firstElementChild;
      expect(root).not.toBeNull();
      expect(root?.className).toBe('xc-shell xc-devices');
      expect(root?.classList.contains('xc-shell')).toBe(true);
      expect(root?.classList.contains('xc-devices')).toBe(true);
      dispose();
    });

    it('renders <div class="xc-shell xc-kiosk"> when look="kiosk"', () => {
      const host = document.createElement('div');
      const dispose = render(
        () => createComponent(AppShell, { title: 'Kiosk', look: 'kiosk' }),
        host,
      );
      const root = host.firstElementChild;
      expect(root).not.toBeNull();
      expect(root?.className).toBe('xc-shell xc-kiosk');
      expect(root?.classList.contains('xc-shell')).toBe(true);
      expect(root?.classList.contains('xc-kiosk')).toBe(true);
      dispose();
    });

    it('renders <div class="xc-shell xc-admin"> when look="admin"', () => {
      const host = document.createElement('div');
      const dispose = render(
        () => createComponent(AppShell, { title: 'Admin', look: 'admin' }),
        host,
      );
      const root = host.firstElementChild;
      expect(root).not.toBeNull();
      expect(root?.className).toBe('xc-shell xc-admin');
      expect(root?.classList.contains('xc-shell')).toBe(true);
      expect(root?.classList.contains('xc-admin')).toBe(true);
      dispose();
    });
  });
});

