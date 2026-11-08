// OL-7 row 105 (verification ~/xc-verify-138 row 105, shot 057): the kiosk's disabled "ตรวจยอด →" (#review) is white
// on light grey (1.65:1). K1's `button:disabled` (kiosk.css, last block) loses to the more specific
// `body.customer-kiosk #review` / `#review:disabled` rules in kiosk-xchange.css, which keep `color: var(--xc-on-brand)`
// and only swap the background to --xc-gray-300. The fix is in kiosk-xchange.css's #review:disabled rule.
// Written before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let xchange = '';
let tokens = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  const dir = here + '../../../api/static/kiosk/';
  xchange = fs.readFileSync(dir + 'kiosk-xchange.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
  tokens = fs.readFileSync(dir + 'xc-tokens.css', 'utf8');
});

/** Every declaration block whose selector list contains exactly `selector` (whitespace-insensitive). */
function blocks(css: string, selector: string): string[] {
  const want = selector.replace(/\s+/g, ' ').trim();
  const out: string[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(css)) !== null) {
    const sels = m[1]!.split(',').map((s) => s.replace(/\s+/g, ' ').trim());
    if (sels.includes(want)) out.push(m[2]!);
  }
  return out;
}

const decl = (body: string, prop: string) =>
  new RegExp(`(?:^|;)\\s*${prop}\\s*:\\s*([^;]+)`).exec(body)?.[1]?.trim();

function hex(name: string): string {
  const v = new RegExp(`--${name}\\s*:\\s*(#[0-9a-fA-F]{6})`).exec(tokens)?.[1];
  if (!v) throw new Error(`token --${name} not found as a 6-digit hex`);
  return v;
}

function contrast(a: string, b: string): number {
  const lum = (h: string) => {
    const c = [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
    const l = c.map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
    return 0.2126 * l[0]! + 0.7152 * l[1]! + 0.0722 * l[2]!;
  };
  const [x, y] = [lum(a), lum(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

const SEL = 'body.customer-kiosk #review:disabled';

describe('KR row 105: the kiosk Review button disabled state (api/static/kiosk/kiosk-xchange.css)', () => {
  it('KR-1 there is exactly one #review:disabled rule', () => {
    expect(blocks(xchange, SEL)).toHaveLength(1);
  });

  it('KR-2 it sets the text colour, not only the background: grey-600 on grey-100', () => {
    const body = blocks(xchange, SEL)[0] ?? '';
    expect(decl(body, 'background')).toBe('var(--xc-gray-100)');
    expect(decl(body, 'color')).toBe('var(--xc-gray-600)');
  });

  it('KR-3 it draws a visible edge (the enabled rule has border:0) and stays opaque', () => {
    const body = blocks(xchange, SEL)[0] ?? '';
    expect(decl(body, 'border')).toBe('1px solid var(--xc-gray-300)');
    expect(decl(body, 'opacity')).toBe('1');
    expect(decl(body, 'cursor')).toBe('not-allowed');
  });

  it('KR-4 the token pair reads at ≥ 4.5:1 (WCAG AA body text)', () => {
    expect(contrast(hex('xc-gray-600'), hex('xc-gray-100'))).toBeGreaterThanOrEqual(4.5);
  });

  it('KR-5 the enabled #review rule is unchanged: brand fill, on-brand text', () => {
    const body = blocks(xchange, 'body.customer-kiosk #review')[0] ?? '';
    expect(decl(body, 'background')).toBe('var(--xc-brand)');
    expect(decl(body, 'color')).toBe('var(--xc-on-brand)');
  });
});
