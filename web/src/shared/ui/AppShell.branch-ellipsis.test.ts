import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(path: string, encoding: string): string;
}

interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

const PHONE = '(max-width:760px)';
const normSel = (s: string) => s.trim().replace(/\s+/g, ' ').replace(/\s*([>+~(),])\s*/g, '$1');

function parseCss(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const norm = (s: string) => s.replace(/\s+/g, '');
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

async function readSibling(name: string): Promise<string> {
  const fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
  const path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
  const here = path.dirname(new URL(import.meta.url).pathname);
  return fs.readFileSync(path.join(here, name), 'utf8');
}

async function sheet(): Promise<Rule[]> {
  return parseCss(await readSibling('shell.css'));
}

describe('AppShell branch ellipsis at phone widths (V3-5, #6520)', () => {
  it('at <= 760px, .account-bar button has display: block and white-space: nowrap', async () => {
    const rules = await sheet();
    const sel = normSel('.account-bar button');
    const matching = rules.filter((r) => r.selector === sel && r.media === PHONE);
    expect(matching.length, 'at least one .account-bar button rule inside <= 760px block').toBeGreaterThan(0);
    // checks every matching rule inside the media block, not just the first one
    const merged = new Map<string, string>();
    for (const r of matching) {
      for (const [k, v] of r.decls) {
        merged.set(k, v);
      }
    }
    expect(merged.get('display'), 'display on phone button').toBe('block');
    expect(merged.get('white-space'), 'white-space on phone button').toBe('nowrap');
    expect(merged.get('overflow'), 'overflow on phone button').toBe('hidden');
    expect(merged.get('text-overflow'), 'text-overflow on phone button').toBe('ellipsis');
    expect(merged.get('min-width'), 'min-width on phone button').toBe('0');
    expect(merged.get('max-width'), 'max-width on phone button').toBe('100%');
  });

  it('at <= 760px, .account-bar > span has min-width: 0, max-width: 100%, overflow: hidden, text-overflow: ellipsis, white-space: nowrap', async () => {
    const rules = await sheet();
    const sel = normSel('.account-bar > span');
    const matching = rules.filter((r) => r.selector === sel && r.media === PHONE);
    expect(matching.length, 'at least one .account-bar > span rule inside <= 760px block').toBeGreaterThan(0);
    // checks every matching rule inside the media block, not just the first one
    const merged = new Map<string, string>();
    for (const r of matching) {
      for (const [k, v] of r.decls) {
        merged.set(k, v);
      }
    }
    expect(merged.get('min-width'), 'min-width on phone span').toBe('0');
    expect(merged.get('max-width'), 'max-width on phone span').toBe('100%');
    expect(merged.get('overflow'), 'overflow on phone span').toBe('hidden');
    expect(merged.get('text-overflow'), 'text-overflow on phone span').toBe('ellipsis');
    expect(merged.get('white-space'), 'white-space on phone span').toBe('nowrap');
  });

  it('above 760px, .account-bar button and span rules remain unchanged', async () => {
    const rules = await sheet();
    const selBtn = normSel('.account-bar button');
    const matchingBtn = rules.filter((r) => r.selector === selBtn && r.media === '');
    expect(matchingBtn.length, 'at least one .account-bar button rule above 760px').toBeGreaterThan(0);
    const mergedBtn = new Map<string, string>();
    for (const r of matchingBtn) {
      for (const [k, v] of r.decls) {
        mergedBtn.set(k, v);
      }
    }
    expect(mergedBtn.get('display'), 'display above 760px').toBe('inline-flex');
    expect(mergedBtn.get('white-space'), 'white-space above 760px').toBe('nowrap');
    expect(mergedBtn.get('overflow'), 'overflow above 760px').toBeUndefined();
    expect(mergedBtn.get('text-overflow'), 'text-overflow above 760px').toBeUndefined();

    const selSpan = normSel('.account-bar > span');
    const matchingSpan = rules.filter((r) => r.selector === selSpan && r.media === '');
    expect(matchingSpan, 'no .account-bar > span rule above 760px').toHaveLength(0);
  });
});
