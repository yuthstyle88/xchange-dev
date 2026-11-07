// OL-6 A4 (audit #564 issuecomment-6702970101 R1; owner ruling issuecomment-6703173879 item 1: bundle Noto Sans Thai
// woff2, 400 + 600): the staff font stack named "XC Noto Sans Thai" but no @font-face existed under web/, so Latin drew in
// Arial and Thai in Tahoma. The variable font (one file per subset covers both weights) comes from the pinned OFL package
// @fontsource-variable/noto-sans-thai, registered as 'Noto Sans Thai Variable'. Customer pages keep their font (ruling A1):
// their own --xc-font-sans does not name the new face, so it is never downloaded there.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let here = '';
let fs: NodeFs;
const read = (rel: string) => fs.readFileSync(here + rel, 'utf8');
const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '');

beforeAll(async () => {
  const specifier = 'node:fs';
  fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
});

describe('A4 the staff brand font is bundled', () => {
  // The import sits in the two staff entry points next to './xc-theme.css', not as a CSS @import in xc-theme.css, whose
  // rule parsers (one-look.test.ts G4, xc-theme.scope.test.ts) read rules only.
  it('A4-1 AppShell.tsx and AuthLayout.tsx import the pinned variable font (weight axis), right before xc-theme.css', () => {
    for (const file of ['AppShell.tsx', 'AuthLayout.tsx']) {
      const src = read(file);
      expect(src, file).toContain("import '@fontsource-variable/noto-sans-thai/wght.css';\nimport './xc-theme.css';");
    }
  });

  it("A4-2 the staff --xc-font-sans starts with 'Noto Sans Thai Variable' and keeps the old fallbacks after it", () => {
    const m = /--xc-font-sans:\s*([^;]+);/.exec(strip(read('xc-theme.css')));
    expect(m, '--xc-font-sans is declared').not.toBeNull();
    const fams = m![1]!.split(',').map((f) => f.trim());
    expect(fams[0]).toBe("'Noto Sans Thai Variable'");
    expect(fams.slice(1)).toEqual([
      "'XC Noto Sans Thai'", "'XC Noto Sans Arabic'", "'XC Noto Sans Devanagari'", "'XC Noto Sans Lao'",
      "'XC Noto Sans Myanmar'", 'Arial', 'Tahoma', 'sans-serif',
    ]);
  });

  it('A4-3 web/package.json pins @fontsource-variable/noto-sans-thai at exactly 5.3.0 under dependencies', () => {
    const pkg = JSON.parse(read('../../../package.json')) as { dependencies?: Record<string, string> };
    expect(pkg.dependencies?.['@fontsource-variable/noto-sans-thai']).toBe('5.3.0');
  });

  it('A4-4 the customer font stack does not name the new face (ruling A1: customer pages unchanged)', () => {
    const cust = strip(read('../../screens/customer/_shell/xc-customer-tokens.css'));
    expect(cust).not.toContain('Noto Sans Thai Variable');
    expect(cust).toMatch(/--xc-font-sans:\s*'XC Noto Sans Thai', Arial, Tahoma, sans-serif;/);
  });
});
