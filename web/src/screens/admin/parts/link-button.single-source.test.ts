// Single source for `.link-button` (v2 admin-html.ts:3 `.ref` + base button border).
// Written before the implementation by the ticket's author (claude/5edf7bb5).
// Do not edit, delete or weaken; add tests in a file of your own.
//
// The shared rule lives in `parts.css` (#919 / PR #932) at specificity (0,4,0). Page stylesheets that
// re-declare `.link-button` at the same specificity make the result depend on stylesheet load order:
// `transactions.css` sets `border: 0` (drops v2's 1px transparent border) and `slips.css` is a byte copy
// of the shared rule. This file pins:
//   - [required] `transactions.css` and `slips.css` contain no `.link-button` selector at all;
//   - [required] `parts.css` still carries the shared rule with v2's values, `border: 1px solid transparent`
//     included.
// The CSS is read as text (vitest loads no CSS), as in web/src/shared/ui/QueueCard.float.test.tsx.
import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}
interface NodePath {
  dirname(p: string): string;
  join(...segments: string[]): string;
}

let fs: NodeFs;
let path: NodePath;
let here: string;

/** Reads a stylesheet relative to this file's directory (`web/src/screens/admin/parts`). */
async function read(rel: string): Promise<string> {
  if (!fs) {
    fs = (await import(/* @vite-ignore */ 'node:fs' as string)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ 'node:path' as string)) as unknown as NodePath;
    here = path.dirname(new URL(import.meta.url).pathname);
  }
  const file = path.join(here, rel);
  expect(fs.existsSync(file), `${rel} exists`).toBe(true);
  return fs.readFileSync(file, 'utf8');
}

/** Rules as { selector, body } with comments stripped and whitespace collapsed. */
function rules(text: string): { selector: string; body: string }[] {
  const src = text.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: { selector: string; body: string }[] = [];
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    out.push({ selector: m[1]!.trim().replace(/\s+/g, ' '), body: m[2]!.replace(/\s+/g, ' ').trim() });
  }
  return out;
}

/** Declarations of a rule body, property → value (last wins). */
function decls(body: string): Map<string, string> {
  const d = new Map<string, string>();
  for (const part of body.split(';')) {
    const k = part.indexOf(':');
    if (k > 0) d.set(part.slice(0, k).trim().toLowerCase(), part.slice(k + 1).trim());
  }
  return d;
}

const linkButtonSelectors = (text: string) =>
  rules(text)
    .map((r) => r.selector)
    .filter((s) => /\.link-button\b/.test(s));

describe('.link-button has one source: parts.css', () => {
  it('transactions.css does not re-declare .link-button', async () => {
    const text = await read('../transactions/transactions.css');
    expect(linkButtonSelectors(text)).toEqual([]);
  });

  it('slips.css does not re-declare .link-button', async () => {
    const text = await read('../slips/slips.css');
    expect(linkButtonSelectors(text)).toEqual([]);
  });

  it('parts.css keeps the shared rule with v2 .ref values and the 1px transparent border', async () => {
    const text = await read('./parts.css');
    const shared = rules(text).filter((r) => r.selector === '.xc-admin .xc-content.xc-content .link-button');
    expect(shared.length, 'exactly one shared .link-button rule').toBe(1);
    const d = decls(shared[0]!.body);
    expect(d.get('border')).toBe('1px solid transparent');
    expect(d.get('background')).toBe('transparent');
    expect(d.get('color')).toBe('var(--brand)');
    expect(d.get('font-weight')).toBe('600');
    expect(d.get('font-size')).toBe('13px');
    expect(d.get('padding')).toBe('0');
    expect(d.get('min-height')).toBe('32px');
    expect(d.get('cursor')).toBe('pointer');
  });
});
