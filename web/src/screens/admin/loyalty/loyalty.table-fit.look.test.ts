// OL-7 admin-loyalty (verifier row 77, VERIFY.md: "the POINT table still overflows: .table-scroll scrollWidth 940 >
// clientWidth 914"). The POINT version table has 10 columns. admin.css draws every admin cell with 24px side padding
// (`.xc-admin .xc-content.xc-content th` 12px 24px, `td` 16px 24px: 480px of padding over 10 columns) and makes every table
// `white-space: nowrap`, so the long Thai column titles ("อายุคะแนน (เดือน)", "อัตราส่วนคะแนน") cannot wrap. The fix stays in
// the screen's own stylesheet: the loyalty tables use 12px side padding and their titles wrap. admin.css is not touched.
// Written before the implementation by claude/94389a6b for the sub-controller claude/1cc51615. Do not edit, delete or
// weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

/** Declarations of the first rule whose selector list contains `selector` exactly. */
function decls(selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(css)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    return out;
  }
  return undefined;
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'loyalty.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
});

// Same weight as admin.css's own cell rules (.xc-admin .xc-content.xc-content th) plus the screen class, so it wins.
const TH = '.xc-admin .xc-content.xc-content .loyalty-screen th';
const TD = '.xc-admin .xc-content.xc-content .loyalty-screen td';

describe('loyalty version tables fit the card', () => {
  it('LT-1 column titles wrap', () => {
    expect(decls(TH)?.get('white-space')).toBe('normal');
  });

  it('LT-2 titles have 12px side padding (admin.css gives 24px)', () => {
    const d = decls(TH);
    expect(d?.get('padding-left')).toBe('12px');
    expect(d?.get('padding-right')).toBe('12px');
  });

  it('LT-3 cells have 12px side padding (admin.css gives 24px)', () => {
    const d = decls(TD);
    expect(d?.get('padding-left')).toBe('12px');
    expect(d?.get('padding-right')).toBe('12px');
  });

  it('LT-4 cells still do not wrap (dates and numbers stay on one line)', () => {
    expect(decls(TD)?.has('white-space') ?? false).toBe(false);
  });
});
