// OL-7 verification-intake (verifier row 101, VERIFY.md: "the intake form still has inline labels with uneven widths
// (รหัสลูกค้า 204px field, ประเภทเอกสาร select, file 337px)"). The branch intake form (VerificationScreen.tsx, the
// `branch-verification-form` panel) is three `<div><label><span>…</span><input|select/></label></div>` blocks and
// verification.css has no rule for them, so each label sits inline beside a field of the browser's own width. They get the
// shared field look (xc-theme.css `.xc-field`: label above, 6px gap, one field per row, 16px between rows) with the fields at
// full width. The file input is left alone: it becomes the shared file-field part in claude-17's ticket this round.
// Written before the implementation by claude/94389a6b for the sub-controller claude/1cc51615. Do not edit, delete or
// weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

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
  css = fs.readFileSync(here + 'verification.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
});

const FORM = '.xc-verification-panel form';

describe('branch intake form fields', () => {
  it('VI-1 each field row is spaced like the shared field (16px apart)', () => {
    expect(decls(`${FORM} > div`)?.get('margin-bottom')).toBe('var(--xc-space-4)');
  });

  it('VI-2 the label stacks its text above its field with a 6px gap', () => {
    const d = decls(`${FORM} > div > label`);
    expect(d?.get('display')).toBe('grid');
    expect(d?.get('gap')).toBe('6px');
  });

  it('VI-3 the label text reads like the shared field label (600 weight, ink)', () => {
    const d = decls(`${FORM} > div > label > span`);
    expect(d?.get('font-weight')).toBe('600');
    expect(d?.get('color')).toBe('var(--xc-ink)');
  });

  it('VI-4 the customer id input and the document type select take the full width; the file input is not styled here', () => {
    expect(decls(`${FORM} > div > label > input:not([type="file"])`)?.get('width')).toBe('100%');
    expect(decls(`${FORM} > div > label > select`)?.get('width')).toBe('100%');
    expect(css).not.toMatch(/\[type="file"\]\s*\{/);
  });
});
