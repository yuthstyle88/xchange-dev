// OL-8 receipt-preview-title (verification r12, "Seen outside the 138 rows"): on /devices-receipt-preview the page
// title (AppShell's `main.xc-shell-main > h1.xc-shell-title`, block, margin 0) spans the whole 992 px main column from
// x=144, while the 360 px slip column is centred (x 460-820; RP-8 pins that centring). The title must sit over the
// column: this page's own CSS gives the shell title the column's width and centres it. Measured in Chromium at 1280:
// title 144-1136 before, 460-820 after, equal to the bar and the card. Written before the fix by claude/883e8664
// (pane claude-3). Do not edit, delete or weaken.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'devices-receipt-preview.css', 'utf8');
});

function decls(selector: string): Map<string, string> | undefined {
  const plain = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(plain)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

const TITLE = '.xc-shell-main:has(> .xc-receipt-preview) > .xc-shell-title';

describe('OL-8 receipt-preview-title: the page title sits over the 360 px slip column', () => {
  it('RPT-1 the shell title on this page is 360 px wide at most and centred', () => {
    const rule = decls(TITLE);
    expect(rule, `rule \`${TITLE}\` in devices-receipt-preview.css`).toBeDefined();
    expect(rule!.get('max-width')).toBe('360px');
    expect(rule!.get('margin-inline')).toBe('auto');
  });

  it('RPT-2 the title width is the same number as the bar and the card width', () => {
    const title = decls(TITLE)?.get('max-width');
    expect(title, 'title max-width').toBe('360px');
    expect(decls('.xc-receipt-preview-bar')?.get('max-width'), 'bar max-width').toBe(title);
    expect(decls('.xc-receipt-preview-card')?.get('max-width'), 'card max-width').toBe(title);
  });

  it('RPT-3 the column stays centred (RP-8): the fix moves the title, not the column', () => {
    expect(decls('.xc-receipt-preview')?.get('justify-items')).toBe('center');
  });
});
