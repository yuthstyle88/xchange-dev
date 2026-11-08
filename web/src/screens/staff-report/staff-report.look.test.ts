// OL-6 wave B staff-report (audit #564 issuecomment-6702970101, shot 013): the whole sheet is bare on the grey page, the
// back link is a browser-default link, footnotes are 12px and the metrics are loose text. The screen keeps printing on
// plain paper. Written before the fix by claude/bbe9ce8b. Do not edit, delete or weaken.
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

function printRule(css: string, selector: string): string {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  return new RegExp(selector.replace(/\./g, '\\.').replace(/ /g, '\\s+') + '\\s*\\{([^}]*)\\}').exec(stripped)?.[1] ?? '';
}

describe('staff-report look', () => {
  it('SR-1 the screen loads the shared card and its parts are cards', async () => {
    const src = await readHere('StaffReport.tsx');
    expect(src).toMatch(/import '\.\.\/\.\.\/shared\/ui\/card\.css';/);
    for (const cls of [
      'staff-report-tools card',
      'staff-report-refusal card',
      'staff-report-heading card',
      'staff-report-table-card card',
      'staff-report-checks card',
      'staff-report-signers card',
    ]) {
      expect(src, cls).toContain(`class="${cls}"`);
    }
  });

  it('SR-2 the page has the desk gutter', async () => {
    expect(decls(await readHere('staff-report.css'), '.staff-report')?.get('padding')).toBe('var(--xc-space-4) 0'); // restated by OL-8 content-width: the shell owns the side gutter
  });

  it('SR-3 each metric is a stat card: label above a large tabular figure', async () => {
    const src = await readHere('StaffReport.tsx');
    expect(src.match(/<p class="staff-report-stat card">/g)?.length).toBe(5);
    expect(src.match(/<span class="staff-report-stat-label">/g)?.length).toBe(5);
    const css = await readHere('staff-report.css');
    const stat = decls(css, '.staff-report-stat');
    expect(stat?.get('display')).toBe('grid');
    expect(stat?.get('gap')).toBe('4px');
    const label = decls(css, '.staff-report-stat-label');
    expect(label?.get('color')).toBe('var(--xc-muted)');
    const figure = decls(css, '.staff-report-stat b');
    expect(figure?.get('font-size')).toBe('var(--xc-text-xl)');
    expect(figure?.get('font-variant-numeric')).toBe('tabular-nums');
  });

  it('SR-4 the back links are buttons, not browser-default links', async () => {
    const src = await readHere('StaffReport.tsx');
    expect(src).toContain(`<a class="xc-btn staff-report-back" href="/staff">{t('back')}</a>`);
    expect(src).toContain(`<a class="xc-btn staff-report-back" href="/staff">{t('signInAgain')}</a>`);
    expect(decls(await readHere('staff-report.css'), '.staff-report-back')?.get('text-decoration')).toBe('none');
  });

  it('SR-5 the table scrolls inside its card and uses the line colour', async () => {
    const css = await readHere('staff-report.css');
    expect(decls(css, '.staff-report-table-card')?.get('overflow-x')).toBe('auto');
    expect(decls(css, '.staff-report-table td')?.get('border-bottom')).toBe('1px solid var(--xc-line)');
    expect(decls(css, '.staff-report-table td')?.get('font-variant-numeric')).toBe('tabular-nums');
  });

  it('SR-6 footnotes are 13px and muted', async () => {
    const d = decls(await readHere('staff-report.css'), '.staff-report-footer');
    expect(d?.get('font-size')).toBe('13px');
    expect(d?.get('color')).toBe('var(--xc-muted)');
  });

  it('SR-7 a Thai eyebrow has no letter spacing', async () => {
    expect(decls(await readHere('staff-report.css'), '.staff-report-eyebrow:lang(th)')?.get('letter-spacing')).toBe('0');
  });

  it('SR-9 the text inside the checks and signers cards has tidy margins', async () => {
    const css = await readHere('staff-report.css');
    for (const sel of ['.staff-report-checks p', '.staff-report-signers p']) {
      expect(decls(css, sel)?.get('margin'), sel).toBe('0');
    }
    for (const sel of ['.staff-report-checks h3', '.staff-report-signers h3']) {
      expect(decls(css, sel)?.get('margin'), sel).toBe('0 0 8px');
    }
  });

  it('SR-8 paper stays plain: no card shadow, no page gutter', async () => {
    const css = await readHere('print.css');
    expect(printRule(css, '.staff-report .card')).toMatch(/box-shadow\s*:\s*none/);
    expect(printRule(css, '.staff-report')).toMatch(/padding\s*:\s*0/);
  });
});
