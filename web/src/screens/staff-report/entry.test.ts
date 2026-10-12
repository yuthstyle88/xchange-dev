// V3-staff-report #1350: the page entry and the print sheet (design docs/design/xchange-v3-wave3-staff-report.md
// B.3 "HTML entry", D-B7, D-B8, B.1 row 21). Written before the implementation by the up-front test writer
// (claude/14cc24bd, on the xchange FE lane controller claude/5edf7bb5's order). Do not edit, delete or weaken these
// tests; add your own in another file.
import { beforeAll, describe, expect, it } from 'vitest';

const html = import.meta.glob<string>('../../../pages/staff-report.html', {
  query: '?raw',
  import: 'default',
  eager: true,
});
// Read from disk (as the LK-6 gate does): vitest empties a `.css?raw` import.
interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}
const fsName = 'node:fs';
const pathName = 'node:path';
let fs: NodeFs;
let path: NodePath;
let DIR: string;

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
  DIR = path.dirname(new URL(import.meta.url).pathname);
});

function src(file: string): string {
  const full = path.join(DIR, file.slice(2));
  expect(fs.existsSync(full), `web/src/screens/staff-report/${file.slice(2)} exists`).toBe(true);
  return fs.readFileSync(full, 'utf8');
}

function page(): Document {
  const raw = html['../../../pages/staff-report.html'];
  expect(raw, 'web/pages/staff-report.html exists').toBeTypeOf('string');
  return new DOMParser().parseFromString(raw!, 'text/html');
}

const metas = (doc: Document, name: string): string[] =>
  Array.from(doc.querySelectorAll(`meta[name="${name}"]`)).map((m) => m.getAttribute('content') ?? '');

describe('V3-staff-report: web/pages/staff-report.html (B.3, D-B7)', () => {
  it('declares xc-path /staff/report with gate anyone, and mounts /src/screens/staff-report/main.tsx', () => {
    const doc = page();
    expect(metas(doc, 'xc-path')).toEqual(['/staff/report']);
    expect(metas(doc, 'xc-gate')).toEqual(['anyone']);
    const scripts = Array.from(doc.querySelectorAll('script[src]')).map((s) => ({
      src: s.getAttribute('src'),
      type: s.getAttribute('type'),
    }));
    expect(scripts).toContainEqual({ src: '/src/screens/staff-report/main.tsx', type: 'module' });
    expect(doc.body.querySelector('[id]'), 'a mount element').not.toBeNull();
  });
});

describe('V3-staff-report: main.tsx renders AppShell (D-B8, LK-1)', () => {
  it('main.tsx imports and renders AppShell itself', () => {
    const main = src('./main.tsx');
    expect(main).toMatch(/(?:^|\n)\s*import\s+[^;]*\bAppShell\b[^;]*from\s+['"][^'"]*AppShell['"]/);
    expect(main).toMatch(/<\s*AppShell\b/);
  });
});

describe('V3-staff-report: the print sheet (B.1 row 21, D-B8)', () => {
  it('print.css sets @page A4 with a 12mm margin inside @media print, and is imported by the screen or its entry', () => {
    const css = src('./print.css').replace(/\/\*[\s\S]*?\*\//g, '');
    expect(css).toMatch(/@media\s+print/);
    expect(css).toMatch(/@page\s*\{[^}]*size\s*:\s*A4/);
    expect(css).toMatch(/@page\s*\{[^}]*margin\s*:\s*12mm/);
    const importers = [src('./main.tsx'), src('./StaffReport.tsx')];
    expect(importers.some((s) => /import\s+['"]\.\/print\.css['"]/.test(s)), 'print.css is imported').toBe(true);
  });

  it('staff-report.css exists and is imported by the screen or its entry (LK-6)', () => {
    src('./staff-report.css');
    const importers = [src('./main.tsx'), src('./StaffReport.tsx')];
    expect(
      importers.some((s) => /import\s+['"]\.\/staff-report\.css['"]/.test(s)),
      'staff-report.css is imported',
    ).toBe(true);
  });
});
