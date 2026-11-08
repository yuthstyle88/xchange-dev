// OL-6 B page-gutter (audit row desk/staff 8: "page gutters 16/32/40/~90px; no single content width"): every page on
// `main.xc-shell-main` gets one content width and one side gutter from shell.css. The width is the mock-up's
// `.page` (docs/design/one-look-mock/index.html:60); the gutters line up with the shell header's (shell.css
// `.xc-shell-header` 24 px, 16 px at 760 px or less). Written before the implementation by claude/88867fb6 for the
// sub-controller claude/1cc51615. Do not edit, delete or weaken; add tests in a file of your own.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'shell.css', 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
});

/** The body of the `@media (max-width: 760px)` block, by matching its braces. */
function phoneBlock(): string {
  const at = css.indexOf('@media (max-width: 760px)');
  expect(at, 'the 760px block exists').toBeGreaterThanOrEqual(0);
  const open = css.indexOf('{', at);
  let depth = 1;
  let i = open + 1;
  while (i < css.length && depth > 0) {
    if (css[i] === '{') depth += 1;
    if (css[i] === '}') depth -= 1;
    i += 1;
  }
  return css.slice(open + 1, i - 1);
}

/** The declarations of the rule whose selector is exactly `.xc-shell-main`, in `source`. */
function mainDecls(source: string): Map<string, string> {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  const out = new Map<string, string>();
  while ((m = re.exec(source)) !== null) {
    if (m[1]!.trim() !== '.xc-shell-main') continue;
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
  }
  return out;
}

/** The base stylesheet, with the media blocks taken out. */
function baseRules(): string {
  const at = css.indexOf('@media');
  return at < 0 ? css : css.slice(0, at);
}

describe('page-gutter: one content width and gutter on .xc-shell-main', () => {
  it('PG-1 the content width is the mock-up page width, centred, with the padding inside it', () => {
    const d = mainDecls(baseRules());
    expect(d.get('max-width')).toBe('1008px'); // restated by OL-8 content-width (960 px column)
    expect(d.get('margin')).toBe('auto');
    expect(d.get('box-sizing')).toBe('border-box');
  });

  it('PG-2 above the phone breakpoint the side gutter is 24 px; top and bottom stay 24 and 48', () => {
    expect(mainDecls(baseRules()).get('padding')).toBe('24px 24px 48px');
  });

  it('PG-3 at 760 px or less the side gutter is 16 px; top and bottom stay 16 and 40', () => {
    expect(mainDecls(phoneBlock()).get('padding')).toBe('16px 16px 40px');
  });

  it('PG-4 the gutters match the shell header on both sides of the breakpoint', () => {
    const rule = (source: string, selector: string): string | undefined => {
      const re = /([^{}]+)\{([^{}]*)\}/g;
      let m: RegExpExecArray | null;
      while ((m = re.exec(source)) !== null) {
        if (m[1]!.trim() === selector) {
          const p = /(?:^|;)\s*padding\s*:\s*([^;]+)/.exec(m[2]!);
          if (p) return p[1]!.trim();
        }
      }
      return undefined;
    };
    expect(rule(baseRules(), '.xc-shell-header')?.split(/\s+/)[1]).toBe('24px');
    expect(rule(phoneBlock(), '.xc-shell-header')?.split(/\s+/)[1]).toBe('16px');
  });
});
