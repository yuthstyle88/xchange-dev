// OL-6 L7b (design docs/design/xchange-one-look-layout.md §3 L7b): every transaction-detail section is a card with
// the mock-up's card values (docs/design/one-look-mock/index.html:39 `.card`, the same tokens as
// web/src/shared/ui/card.css `.xc-theme .card`). Owner 2026-11-07 ~12:20 ICT on the desk forms: "มันควรทำเป็นการ์ดนะ".
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
  css = fs.readFileSync(here + 'desk-transaction.css', 'utf8');
});

describe('L7b transaction-detail sections are cards', () => {
  it('L7b-1 .xc-dtx-section has the card surface, line, radius, shadow and 20px padding', () => {
    const d = decls(css, '.xc-dtx-section');
    expect(d, '.xc-dtx-section rule exists').toBeDefined();
    expect(d?.get('background')).toBe('var(--xc-surface)');
    expect(d?.get('border')).toBe('1px solid var(--xc-line)');
    expect(d?.get('border-radius')).toBe('var(--xc-radius-xl)');
    expect(d?.get('box-shadow')).toBe('var(--xc-shadow-sm)');
    expect(d?.get('padding')).toBe('20px');
  });

  it('L7b-2 at 800 px or less a section pads 16px', () => {
    const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
    const block = /@media\s*\(max-width:\s*800px\)\s*\{([\s\S]*?\})\s*\}/.exec(stripped)?.[1] ?? '';
    const rule = /\.xc-dtx-section\s*\{([^}]*)\}/.exec(block)?.[1] ?? '';
    expect(rule, '.xc-dtx-section inside @media (max-width: 800px)').not.toBe('');
    expect(rule).toMatch(/padding\s*:\s*16px\s*(;|$)/);
  });
});
