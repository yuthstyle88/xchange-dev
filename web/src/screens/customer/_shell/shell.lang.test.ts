// UX-4 (#6463): pins customer language button size (44px target), no overlap,
// no XC header mark, and right alignment. Parses customer/_shell/shell.css
// the way web/src/shared/ui/QueueCard.float.test.tsx does.
import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
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

async function readSibling(name: string): Promise<string | undefined> {
  if (!fs) {
    fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
    here = path.dirname(new URL(import.meta.url).pathname);
  }
  const file = path.join(here, name);
  return fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : undefined;
}

async function sheet(): Promise<Rule[]> {
  const text = await readSibling('shell.css');
  expect(text, 'web/src/screens/customer/_shell/shell.css exists').toBeDefined();
  return parseCss(text!);
}

describe('customer/_shell/shell.css (UX-4 #6463)', () => {
  it('pins no XC mark on the customer header title::before', async () => {
    const rules = await sheet();
    expect(value(rules, '.xc-customer .xc-shell-title::before', 'content')).toBe('none');
  });

  it('pins customer language button minimum target 44px', async () => {
    const rules = await sheet();
    expect(value(rules, '.xc-customer .xc-shell-header > button', 'min-height')).toBe('44px');
    expect(value(rules, '.xc-customer .xc-shell-header > button', 'min-width')).toBe('44px');
  });

  it('pins no negative margin-left on language buttons', async () => {
    const rules = await sheet();
    const adjacentMarginLeft = value(rules, '.xc-customer .xc-shell-header > button + button', 'margin-left');
    expect(adjacentMarginLeft).toBe('0');
    expect(adjacentMarginLeft).not.toMatch(/-/);
  });

  it('pins switch position on the right via order or margin-left: auto', async () => {
    const rules = await sheet();
    const btnOrder = value(rules, '.xc-customer .xc-shell-header > button', 'order');
    const autoMargin = value(rules, '.xc-customer .xc-shell-header > .xc-shell-title + button', 'margin-left');
    expect(btnOrder === '2' || autoMargin === 'auto').toBe(true);
    expect(btnOrder).toBe('2');
    expect(autoMargin).toBe('auto');
  });
});
