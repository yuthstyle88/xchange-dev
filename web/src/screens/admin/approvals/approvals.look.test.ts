// OL-6 B admin-approvals (audit #564 issuecomment-6702970101, shot 040): two unlabelled grey chip rows where the
// selected chip was green text only, and a bare oversized "0 / 500" counter.
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

import * as table from './i18n';

const P = '.xc-admin .xc-content.xc-content .approvals-filter-row';

describe('approvals look', () => {
  it('AP-1 both filter rows sit in one card, each with a visible label', () => {
    const tsx = readHere('Approvals.tsx');
    expect(tsx).toContain('<div class="approvals-filters card">');
    expect(tsx.match(/<div class="approvals-filter-row">/g)).toHaveLength(2);
    expect(tsx).toContain(`<span class="approvals-filter-label">{t('filterStatus')}</span>`);
    expect(tsx).toContain(`<span class="approvals-filter-label">{t('filterType')}</span>`);
    expect(tsx.indexOf("t('filterStatus')")).toBeLessThan(tsx.indexOf('data-status-switch>'));
    expect(tsx.indexOf("t('filterType')")).toBeLessThan(tsx.indexOf('data-type-switch>'));
    expect(table.th.filterStatus).toBe('สถานะ');
    expect(table.th.filterType).toBe('ประเภท');
    expect(table.en.filterStatus).toBe('Status');
    expect(table.en.filterType).toBe('Type');
  });

  it('AP-2 the filter card and rows', () => {
    const css = readHere('approvals.css');
    expect(decls(css, '.xc-admin .xc-content .approvals-filters')?.get('padding')).toBe('16px 20px');
    const row = decls(css, '.approvals-filter-row');
    expect(row?.get('display')).toBe('grid');
    expect(row?.get('grid-template-columns')).toBe('72px minmax(0, 1fr)');
    expect(decls(css, '.approvals-filter-label')?.get('color')).toBe('var(--xc-muted)');
  });

  it('AP-3 a chip is a white pill with a line; the pressed chip is brand-filled', () => {
    const css = readHere('approvals.css');
    const chip = decls(css, `${P} button.pill`);
    expect(chip?.get('border')).toBe('1px solid var(--xc-line)');
    expect(chip?.get('border-radius')).toBe('var(--xc-radius-pill)');
    expect(chip?.get('background')).toBe('var(--xc-surface)');
    expect(chip?.get('min-height')).toBe('36px');
    const on = decls(css, `${P} button.pill[aria-pressed="true"]`);
    expect(on?.get('background')).toBe('var(--xc-brand)');
    expect(on?.get('border-color')).toBe('var(--xc-brand)');
    expect(on?.get('color')).toBe('var(--xc-on-brand)');
  });

  it('AP-4 the reason counter is small, muted and right-aligned', () => {
    expect(readHere('Approvals.tsx')).toContain('<div class="xc-field-count">{reason().length} / 500</div>');
    // Restated by the sub-controller for OL-8 reuse-field-count: the rule moved to shared/ui/field-count.css.
    const d = decls(readHere('../../../shared/ui/field-count.css'), '.xc-field-count');
    expect(d?.get('font-size')).toBe('var(--xc-text-sm)');
    expect(d?.get('color')).toBe('var(--xc-muted)');
    expect(d?.get('text-align')).toBe('right');
  });

  it('AP-5 at 800px or less the label sits above its chips', () => {
    const m = media(readHere('approvals.css'), 'max-width: 800px');
    expect(decls(m, '.approvals-filter-row')?.get('grid-template-columns')).toBe('1fr');
  });
});
