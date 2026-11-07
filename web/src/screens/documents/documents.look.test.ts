// OL-6 wave B documents (audit #564 issuecomment-6702970101, shots 016-017, 051-052): the document list is a bulleted
// list of underlined links on the grey page and the not-found state is bare red text. Written before the fix by
// claude/bbe9ce8b. Do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

async function readHere(file: string): Promise<string> {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  return fs.readFileSync(here + file, 'utf8');
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim());
    }
    return out;
  }
  return undefined;
}

describe('documents look', () => {
  it('DC-1 the screen loads the shared card; both list forms and the refusal are cards', async () => {
    const src = await readHere('DocumentsScreen.tsx');
    expect(src).toMatch(/import '\.\.\/\.\.\/shared\/ui\/card\.css';/);
    expect(src).toContain('<ul class="documents-list card">');
    expect(src).toContain('<div class="documents-list card">');
    expect(src).toContain('<p class="documents-refusal card" role="alert">');
  });

  it('DC-2 the page has the desk gutter, stacks with a gap and drops the 18px text', async () => {
    const d = decls(await readHere('documents.css'), '.documents-screen');
    expect(d?.get('display')).toBe('grid');
    expect(d?.get('gap')).toBe('16px');
    expect(d?.get('padding')).toBe('16px');
    expect(d?.has('font-size')).toBe(false);
  });

  it('DC-3 the list has no bullets and its rows have a line between them', async () => {
    const css = await readHere('documents.css');
    const list = decls(css, '.documents-screen .documents-list');
    expect(list?.get('list-style')).toBe('none');
    expect(list?.get('margin')).toBe('0');
    const row = decls(css, '.documents-screen .documents-list [data-kind]');
    expect(row?.get('margin')).toBe('0');
    expect(row?.get('padding')).toBe('12px 0');
    expect(row?.get('border-top')).toBe('1px solid var(--xc-line)');
    const first = decls(css, '.documents-screen .documents-list [data-kind]:first-child');
    expect(first?.get('padding-top')).toBe('0');
    expect(first?.get('border-top')).toBe('0');
  });

  it('DC-4 links are not underlined', async () => {
    const d = decls(await readHere('documents.css'), '.documents-screen a');
    expect(d?.get('color')).toBe('var(--xc-brand)');
    expect(d?.get('text-decoration')).toBe('none');
    expect(d?.get('font-weight')).toBe('600');
  });

  it('DC-5 the back link is a button at its natural width', async () => {
    expect(await readHere('DocumentsScreen.tsx')).toContain('<a class="documents-back xc-btn" href=');
    const d = decls(await readHere('documents.css'), '.documents-screen .documents-back');
    expect(d?.get('justify-self')).toBe('start');
    expect(d?.has('margin-top')).toBe(false);
  });

  it('DC-6 the refusal keeps the danger colour and loses its stray margin', async () => {
    const d = decls(await readHere('documents.css'), '.documents-screen .documents-refusal');
    expect(d?.get('color')).toBe('var(--xc-danger)');
    expect(d?.get('margin')).toBe('0');
  });
});
