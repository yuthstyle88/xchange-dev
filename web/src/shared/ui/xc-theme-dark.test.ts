// Design §2.6: the dark mode is scoped to staff surfaces on screen, and every text pair reads at 4.5:1.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

let theme = '';
let dark = '';

beforeAll(async () => {
  const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  const here = path.dirname(new URL(import.meta.url).pathname);
  theme = fs.readFileSync(path.join(here, 'xc-theme.css'), 'utf8');
  dark = fs.readFileSync(path.join(here, 'xc-theme-dark.css'), 'utf8');
});

/** The dark value a token takes: xc-theme-dark.css maps --x to var(--xc-dk-…), whose hex sits in xc-theme.css. */
function darkHex(token: string): string {
  const m = dark.match(new RegExp(`--${token}:\\s*var\\(--(xc-dk-[a-z0-9-]+)\\)`));
  if (!m) throw new Error(`no dark mapping for --${token}`);
  const v = theme.match(new RegExp(`--${m[1]}:\\s*(#[0-9a-fA-F]{6})`));
  if (!v) throw new Error(`no value for --${m[1]}`);
  return v[1].toLowerCase();
}

function lum(h: string): number {
  const c = [1, 3, 5]
    .map((i) => parseInt(h.slice(i, i + 2), 16) / 255)
    .map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0]! + 0.7152 * c[1]! + 0.0722 * c[2]!;
}

function ratio(a: string, b: string): number {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x! + 0.05) / (y! + 0.05);
}

describe('xc-theme-dark.css (design §2.6)', () => {
  // Restated 2026-11-20 (owner "ไม่มี dark mode" on the kiosk/customer screens): the scope was staff-only
  // (`.xc-theme:not(.xc-customer):not(.xc-customer *)`); customer surfaces now follow too. Still screen-only.
  it('scopes the dark values to every .xc-theme surface (staff and customer), on screen only', () => {
    const at = dark.indexOf(':root[data-theme="dark"] .xc-theme {');
    expect(at).toBeGreaterThan(0);
    expect(dark.slice(0, at)).toMatch(/@media screen \{\s*$/);
  });

  it('keeps the light values when no mode is set (no :root default)', () => {
    expect(dark).not.toMatch(/:root\s*\{/);
    expect(dark).not.toMatch(/prefers-color-scheme/);
  });

  it.each([
    ['ink on the page', 'xc-gray-900', 'xc-gray-75'],
    ['ink on a surface', 'xc-gray-900', 'xc-gray-0'],
    ['muted on a surface', 'xc-gray-600', 'xc-gray-0'],
    ['brand text on the page', 'xc-brand-text', 'xc-gray-75'],
    ['brand text on a surface', 'xc-brand-text', 'xc-gray-0'],
    ['white on the brand fill', 'xc-on-brand', 'xc-brand-700'],
    ['white on the brand hover', 'xc-on-brand', 'xc-brand-900'],
    ['ok text on a surface', 'xc-green-700', 'xc-gray-0'],
    ['warn text on a surface', 'xc-amber-700', 'xc-gray-0'],
    ['danger text on a surface', 'xc-red-700', 'xc-gray-0'],
  ])('%s reads at 4.5:1 or better', (_label, fg, bg) => {
    expect(ratio(darkHex(fg), darkHex(bg))).toBeGreaterThanOrEqual(4.5);
  });
});
