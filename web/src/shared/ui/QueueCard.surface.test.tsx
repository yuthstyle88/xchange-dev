// V3-2 (#6510, one-look): queue card surface look and long branch name wrapping in queue-card.css.
// Pins:
// 1. .queue-card.xc-theme raises the card look above .xc-theme (background: var(--xc-surface), color, font).
// 2. .queue-card-title has overflow-wrap: anywhere and min-width: 0 so unbroken names wrap inside the card.
// Parsed the way web/src/shared/ui/QueueCard.float.test.tsx does.
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
  const text = await readSibling('queue-card.css');
  expect(text, 'web/src/shared/ui/queue-card.css exists').toBeDefined();
  return parseCss(text!);
}

describe('QueueCard surface and title wrapping (V3-2 #6510)', () => {
  it('pins card background and look above theme root on .queue-card.xc-theme', async () => {
    const rules = await sheet();
    expect(value(rules, '.queue-card.xc-theme', 'background')).toBe('var(--xc-surface)');
    expect(value(rules, '.queue-card.xc-theme', 'color')).toBe('var(--xc-ink)');
    expect(value(rules, '.queue-card.xc-theme', 'font')).toBe('14px/1.5 var(--xc-font-sans)');
  });

  it('pins unbroken title wrapping on .queue-card-title', async () => {
    const rules = await sheet();
    expect(value(rules, '.queue-card-title', 'overflow-wrap')).toBe('anywhere');
    expect(value(rules, '.queue-card-title', 'min-width')).toBe('0');
  });
});
