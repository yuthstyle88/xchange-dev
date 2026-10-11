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

function hasShellOrAuthLayoutImport(source: string): boolean {
  return (
    /(?:^|\n)\s*import\s+[^;]*from\s+['"][^'"]*(?:AppShell|AuthLayout)['"]/.test(source) ||
    /(?:^|\n)\s*import\s+['"][^'"]*(?:AppShell|AuthLayout)['"]/.test(source)
  );
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
      const scriptSource = fs.readFileSync(scriptFullPath, 'utf-8');

      if (hasShellOrAuthLayoutImport(scriptSource)) {
        // Direct import in entry module
        continue;
      }

      // Check explicit child module listing
      const childModules = CHILD_MODULE_ENTRIES[file];
      expect(
        childModules && childModules.length > 0,
        `${file} does not directly import AppShell or AuthLayout, so it must be explicitly listed in CHILD_MODULE_ENTRIES`,
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

        // Assert that the child module imports AppShell or AuthLayout
        const childSource = fs.readFileSync(childFullPath, 'utf-8');
        expect(
          hasShellOrAuthLayoutImport(childSource),
          `Child module ${childRel} reached from ${file} must import AppShell or AuthLayout`,
        ).toBe(true);
      }
    }
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

