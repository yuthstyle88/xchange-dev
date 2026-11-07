// OL-6 L6 (#6546): staff header at tablet widths (761-1279px) and 320px
// Pins rows 1, 3, 4 and 5 of #6546 §9 by parsing shell.css and admin.css as QueueCard.float.test.tsx does.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

/** Flat rules of a stylesheet: one entry per selector of each rule, inside at most one @media. */
function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const norm = (s: string) => s.replace(/\s+/g, '');
  const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const block = (body: string, media: string) => {
    const re = /([^{}]+)\{([^{}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body)) !== null) {
      const decls = new Map<string, string>();
      for (const d of m[2]!.split(';')) {
        const k = d.indexOf(':');
        if (k > 0) decls.set(d.slice(0, k).trim().toLowerCase(), d.slice(k + 1).trim().replace(/\s+/g, ' '));
      }
      for (const sel of m[1]!.split(',')) out.push({ media, selector: normSel(sel), decls });
    }
  };
  let rest = '';
  let i = 0;
  while (i < src.length) {
    const at = src.indexOf('@media', i);
    if (at < 0) {
      rest += src.slice(i);
      break;
    }
    rest += src.slice(i, at);
    const open = src.indexOf('{', at);
    let depth = 1;
    let j = open + 1;
    while (j < src.length && depth > 0) {
      if (src[j] === '{') depth += 1;
      if (src[j] === '}') depth -= 1;
      j += 1;
    }
    block(src.slice(open + 1, j - 1), norm(src.slice(at + 6, open)));
    i = j;
  }
  block(rest, '');
  return out;
}

/** The value of `prop` for exactly `selector` under `media` ('' = no media), the last one winning. */
function value(rules: Rule[], selector: string, prop: string, media = ''): string | undefined {
  const sel = selector.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');
  const med = media.replace(/\s+/g, '');
  let v: string | undefined;
  for (const r of rules) if (r.selector === sel && r.media === med && r.decls.has(prop)) v = r.decls.get(prop);
  return v;
}

let fs: NodeFs;
let path: NodePath;
let here: string;

async function sheet(relative: string): Promise<Rule[]> {
  if (!fs) {
    fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
    here = path.dirname(new URL(import.meta.url).pathname);
  }
  return parseCss(fs.readFileSync(path.join(here, relative), 'utf8'));
}

beforeAll(async () => {
  fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  here = path.dirname(new URL(import.meta.url).pathname);
});

describe('OL-6 L6 header behaviour (#6546 §9)', () => {
  it('row 1: nav link above 760px has white-space: nowrap', async () => {
    const rules = await sheet('shell.css');
    expect(value(rules, '.xc-shell-nav a', 'white-space', ''), 'nav links stay on one line each').toBe('nowrap');
  });

  it('row 3: admin header has min-height and no fixed height', async () => {
    const rules = await sheet('../../screens/admin/admin.css');
    const minHeight = value(rules, '.xc-admin .xc-shell-header', 'min-height', '');
    const height = value(rules, '.xc-admin .xc-shell-header', 'height', '');
    expect(minHeight, 'admin header follows content with min-height').toBeDefined();
    expect(height, 'admin header has no fixed height').toBeUndefined();
  });

  it('row 4: <= 360px header row: the brand name can shrink or ellipsize', async () => {
    const rules = await sheet('shell.css');
    const media = '(max-width:360px)';
    expect(value(rules, '.xc-shell-brand', 'min-width', media), 'brand can shrink below content size').toBe('0');
    expect(value(rules, '.xc-shell-brand', 'overflow', media), 'brand clips overflowing text').toBe('hidden');
    expect(value(rules, '.xc-shell-brand', 'text-overflow', media), 'brand ellipsizes').toBe('ellipsis');
  });

  it('row 5: panel nav rows fill 100% width of the panel', async () => {
    const rules = await sheet('shell.css');
    const media = '(max-width:760px)';
    expect(value(rules, '.xc-shell-panel .xc-shell-nav a', 'width', media), 'panel nav links fill 100% width').toBe('100%');
  });
});
