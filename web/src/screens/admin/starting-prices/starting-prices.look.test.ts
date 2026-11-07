// OL-6 wave B admin-starting-prices (audit #564 issuecomment-6702970101, shot 026):
// - "เพิ่มสกุลเงิน" was grey-filled (gray-75), unlike every other admin secondary (white + 1px line), and broke
//   mid-word ("เพิ่มสกุล / เงิน") because the full-width code field squeezed it. It is now white, keeps its width and
//   stays on one line; its hover is the brand tint the other admin secondaries use.
// - "บันทึกราคา" disabled kept `opacity: 0.5` on top of admin.css's solid disabled colours (A3), so it read as faded;
//   it is now opaque.
// Written before the fix by claude/bbe9ce8b for the OL-6 sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    if (m[1]!.trim() !== selector) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

let css = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'starting-prices.css', 'utf8');
});

describe('admin-starting-prices: the add-currency button is a white secondary on one line', () => {
  it('SP-1 .xc-sp-add-code-btn: white surface, 1px line, no grey fill', () => {
    const d = decls(css, '.xc-sp-add-code-btn');
    expect(d, '.xc-sp-add-code-btn').toBeDefined();
    expect(d!.get('background-color')).toBe('var(--xc-surface)');
    expect(d!.get('border')).toBe('1px solid var(--xc-line)');
  });

  it('SP-2 .xc-sp-add-code-btn keeps its width and its text on one line', () => {
    const d = decls(css, '.xc-sp-add-code-btn')!;
    expect(d.get('flex')).toBe('none');
    expect(d.get('white-space')).toBe('nowrap');
  });

  it('SP-3 its hover is the brand tint, not a darker grey', () => {
    const d = decls(css, '.xc-sp-add-code-btn:hover');
    expect(d, '.xc-sp-add-code-btn:hover').toBeDefined();
    expect(d!.get('background-color')).toBe('var(--xc-brand-soft)');
  });
});

describe('admin-starting-prices: a disabled save is solid', () => {
  it('SP-4 .xc-sp-save-btn:disabled is opaque (opacity 1) and still not-allowed', () => {
    const d = decls(css, '.xc-sp-save-btn:disabled');
    expect(d, '.xc-sp-save-btn:disabled').toBeDefined();
    expect(d!.get('opacity')).toBe('1');
    expect(d!.get('cursor')).toBe('not-allowed');
  });
});
