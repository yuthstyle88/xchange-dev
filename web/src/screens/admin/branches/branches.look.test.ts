// OL-6 B admin-branches (audit #564 issuecomment-6702970101, shot 046): the branch picker label wrapped, the shift
// editor stacked five unlabelled full-width fields and an orphan "–", and the four sections copied card values.
import { beforeAll, describe, expect, it } from 'vitest';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let readHere: (name: string) => string = () => '';

function stripped(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, '');
}

function decls(css: string, selector: string): Map<string, string> | undefined {
  const re = /([^{}]+)\{([^{}]*)\}/g;
  const src = stripped(css);
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    if (!m[1]!.split(',').map((s) => s.trim()).includes(selector)) continue;
    const out = new Map<string, string>();
    for (const d of m[2]!.split(';')) {
      const i = d.indexOf(':');
      if (i > 0) out.set(d.slice(0, i).trim().toLowerCase(), d.slice(i + 1).trim().replace(/\s+/g, ' '));
    }
    return out;
  }
  return undefined;
}

function media(css: string, query: string): string {
  const src = stripped(css);
  const at = src.indexOf(`@media (${query})`);
  if (at < 0) return '';
  const open = src.indexOf('{', at);
  let depth = 1;
  let i = open + 1;
  while (i < src.length && depth > 0) {
    if (src[i] === '{') depth += 1;
    if (src[i] === '}') depth -= 1;
    i += 1;
  }
  return src.slice(open + 1, i - 1);
}

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  readHere = (name) => fs.readFileSync(here + name, 'utf8');
});

const shiftsTsx = () => readHere('ShiftsEditor.tsx');

describe('branches look', () => {
  it('BR-1 the branch picker label stays on one line', () => {
    expect(decls(readHere('branches.css'), '.branches-select-group .branches-label')?.get('white-space')).toBe('nowrap');
  });

  it('BR-2 the four sections use the shared card; .branches-card keeps only 20px padding', () => {
    const css = readHere('branches.css');
    const d = decls(css, '.xc-admin .xc-content .branches-card');
    expect(d?.get('padding')).toBe('20px');
    expect(d?.get('margin-bottom')).toBe('0');
    expect(decls(css, '.branches-card'), 'the old copy of the card values is gone').toBeUndefined();
    expect(readHere('BranchesScreen.tsx')).toContain("import '../../../shared/ui/card.css';");
    expect(readHere('BranchesScreen.tsx')).toContain('<div class="branches-card card">');
    expect(shiftsTsx()).toContain('<section class="branches-card card branches-shifts"');
    expect(readHere('BranchTablets.tsx')).toContain('<section class="branches-card card branches-tablets"');
    expect(readHere('BranchCounters.tsx')).toContain('<section class="branches-card card branches-counters"');
  });

  it('BR-3 a shift row is one grid line: name, four times, remove', () => {
    const d = decls(readHere('branches.css'), '.branches-shifts-row-fields');
    expect(d?.get('display')).toBe('grid');
    expect(d?.get('grid-template-columns')).toBe('minmax(140px, 1.4fr) repeat(4, minmax(110px, 1fr)) auto');
    expect(d?.get('align-items')).toBe('end');
  });

  it('BR-4 every shift field has a visible label above it', () => {
    const tsx = shiftsTsx();
    expect(tsx.match(/<label class="branches-shift-field">/g)).toHaveLength(5);
    for (const key of ['shiftName', 'shiftStart', 'shiftEnd', 'shiftBreakStart', 'shiftBreakEnd']) {
      expect(tsx).toContain(`<span class="branches-shift-label">{t('${key}')}</span>`);
    }
    const css = readHere('branches.css');
    expect(decls(css, '.xc-admin .xc-content.xc-content .branches-shift-field')?.get('flex-direction')).toBe('column');
    expect(decls(css, '.branches-shift-label')?.get('color')).toBe('var(--xc-muted)');
    expect(decls(css, '.xc-admin .xc-content.xc-content .branches-shift-field input')?.get('margin-top')).toBe('0');
  });

  it('BR-5 no orphan dash between start and end', () => {
    expect(shiftsTsx()).not.toContain('<span>–</span>');
  });

  it('BR-6 at 800px or less a shift row is two columns, the name full width', () => {
    const m = media(readHere('branches.css'), 'max-width: 800px');
    expect(decls(m, '.branches-shifts-row-fields')?.get('grid-template-columns')).toBe('repeat(2, minmax(0, 1fr))');
    expect(decls(m, '.branches-shifts-row-fields .branches-shift-field:first-child')?.get('grid-column')).toBe('1 / -1');
  });
});
