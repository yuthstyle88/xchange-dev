// OL-6 wave B desk-form-width (audit #564 issuecomment-6702970101, measured row: step 04 at 1280, `.desk-form` 560 px in
// an ~872 px card leaves 312 px empty). Written before the fix by the ticket author claude/studio-claude-17 for the
// sub-controller claude/1cc51615. Do not edit, delete or weaken.
//
// jsdom lays nothing out, so this pins the CSS. The two columns are scoped to the desk steps (`.desk-step-content`,
// DeskPage.tsx) because /desk/backdated also uses `.desk-form` and lays out its own rows (backdated.css).
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

type Rule = { media: string; selector: string; decls: Map<string, string> };

function parse(text: string): Rule[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Rule[] = [];
  const block = (body: string, media: string) => {
    const re = /([^{}]+)\{([^{}]*)\}/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(body)) !== null) {
      const decls = new Map<string, string>();
      for (const d of m[2]!.split(';')) {
        const i = d.indexOf(':');
        if (i > 0) decls.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
      }
      for (const s of m[1]!.split(',')) out.push({ media, selector: s.trim().replace(/\s+/g, ' '), decls });
    }
  };
  let rest = src;
  for (;;) {
    const at = rest.indexOf('@media');
    if (at < 0) break;
    const open = rest.indexOf('{', at);
    let depth = 1;
    let i = open + 1;
    while (depth > 0 && i < rest.length) {
      if (rest[i] === '{') depth++;
      if (rest[i] === '}') depth--;
      i++;
    }
    block(rest.slice(open + 1, i - 1), rest.slice(at + 6, open).replace(/\s+/g, ''));
    rest = rest.slice(0, at) + rest.slice(i);
  }
  block(rest, '');
  return out;
}

let rules: Rule[] = [];
let backdated = '';
const WIDE = '(min-width:1100px)';
const get = (selector: string, media = '') => rules.find((r) => r.selector === selector && r.media === media)?.decls;

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  rules = parse(fs.readFileSync(here + 'desk.css', 'utf8'));
  backdated = fs.readFileSync(here + '../desk-backdated/BackdatedScreen.tsx', 'utf8');
});

describe('OL-6 B a desk step form fills its card', () => {
  it('FW-1 .desk-form has no width limit and stays one flex column by default', () => {
    const d = get('.desk-form');
    expect(d, '.desk-form rule').toBeDefined();
    expect(d?.has('max-width')).toBe(false);
    expect(d?.get('display')).toBe('flex');
    expect(d?.get('flex-direction')).toBe('column');
  });

  it('FW-2 at 1100px or wider a step form is two equal columns', () => {
    const d = get('.desk-step-content .desk-form', WIDE);
    expect(d?.get('display')).toBe('grid');
    expect(d?.get('grid-template-columns')).toBe('repeat(2, minmax(0, 1fr))');
    expect(d?.get('gap')).toBe('12px 16px');
    expect(d?.get('align-items')).toBe('start');
  });

  it('FW-3 in the two columns only a field takes one column; headings, notices, checks and actions span both', () => {
    expect(get('.desk-step-content .desk-form > *', WIDE)?.get('grid-column')).toBe('1 / -1');
    const field = get('.desk-step-content .desk-form > .desk-field', WIDE);
    expect(field?.get('grid-column')).toBe('auto');
    expect(field?.get('min-width')).toBe('0');
  });

  it('FW-4 no rule makes a phone or tablet form two columns', () => {
    const grid = rules.filter((r) => r.selector.includes('.desk-form') && r.decls.get('display') === 'grid');
    expect(grid.map((r) => r.media)).toEqual([WIDE]);
  });

  it('FW-5 /desk/backdated is not inside .desk-step-content, so its form keeps its own rows', () => {
    expect(backdated).toContain('desk-form backdated-form');
    expect(backdated).not.toContain('desk-step-content');
  });
});
