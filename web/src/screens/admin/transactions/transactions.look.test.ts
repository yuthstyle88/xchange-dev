// OL-6 wave B admin-transactions (audit #564 issuecomment-6702970101, shot 019): the filter row's actions.
// - When the filter row wraps (≤ ~1150 px), "ส่งออกรายการ CSV" + "+ ทำรายการแลกเงิน" dropped to the LEFT under the
//   filters, while Overview keeps them at the right; they now keep to the right end of their row (`margin-left: auto`).
// - The two buttons were 40 px tall beside 44 px fields (mock: 44 px controls), so their row did not line up.
// - The "พบ N รายการ" line sat at 24 px (17 px fields at ≤720 px used 17 px), one px off the filters' 23 px inset.
// Written before the fix by claude/bbe9ce8b for the OL-6 sub-controller claude/1cc51615. Do not edit, delete or weaken.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

/** The declarations of the rule whose selector text is exactly `selector`, at top level or inside `@media <media>`. */
function decls(css: string, selector: string, media?: string): Map<string, string> | undefined {
  let text = css.replace(/\/\*[\s\S]*?\*\//g, '');
  if (media !== undefined) {
    const at = text.indexOf(`@media ${media}`);
    if (at < 0) return undefined;
    let depth = 0;
    let start = -1;
    let end = -1;
    for (let i = at; i < text.length; i++) {
      if (text[i] === '{') {
        if (depth === 0) start = i + 1;
        depth++;
      } else if (text[i] === '}') {
        depth--;
        if (depth === 0) {
          end = i;
          break;
        }
      }
    }
    text = text.slice(start, end);
  } else {
    text = text.replace(/@media[^{]*\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g, '');
  }
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
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
  css = fs.readFileSync(here + 'transactions.css', 'utf8');
});

const S = '.xc-admin .xc-content .transactions-screen';

describe('admin-transactions: the filter row actions', () => {
  it('TX-1 the actions keep to the right end of their row (margin-left: auto) and still wrap', () => {
    const d = decls(css, `${S} .actions-group`);
    expect(d, `${S} .actions-group`).toBeDefined();
    expect(d!.get('margin-left')).toBe('auto');
    expect(d!.get('display')).toBe('flex');
    expect(d!.get('flex-wrap')).toBe('wrap');
  });

  it('TX-2 the action buttons are 44px tall like the fields beside them, padded 9px 16px', () => {
    const d = decls(css, `${S} .actions-group button`);
    expect(d, `${S} .actions-group button`).toBeDefined();
    expect(d!.get('min-height')).toBe('44px');
    expect(d!.get('padding')).toBe('9px 16px');
  });

  it('TX-3 at ≤720px the actions still take the full row from the left (unchanged)', () => {
    const d = decls(css, `${S} .actions-group`, '(max-width: 720px)');
    expect(d, `@media (max-width: 720px) ${S} .actions-group`).toBeDefined();
    expect(d!.get('width')).toBe('100%');
    expect(d!.get('justify-content')).toBe('flex-start');
  });
});

describe('admin-transactions: the count line lines up with the filters', () => {
  it("TX-4 .table-meta uses the filters' 23px inset", () => {
    const filters = decls(css, `${S} .filters`);
    expect(filters!.get('padding')).toBe('23px');
    const d = decls(css, `${S} .table-meta`);
    expect(d, `${S} .table-meta`).toBeDefined();
    expect(d!.get('padding')).toBe('0 23px 14px');
  });

  it("TX-5 at ≤720px .table-meta uses the filters' 17px inset", () => {
    const filters = decls(css, `${S} .filters`, '(max-width: 720px)');
    expect(filters!.get('padding')).toBe('17px');
    const d = decls(css, `${S} .table-meta`, '(max-width: 720px)');
    expect(d, `@media (max-width: 720px) ${S} .table-meta`).toBeDefined();
    expect(d!.get('padding')).toBe('0 17px 14px');
  });
});
