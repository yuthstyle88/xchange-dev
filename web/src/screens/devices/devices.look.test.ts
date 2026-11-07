// OL-6 B devices (audit #564 issuecomment-6702970101, shot 053): near-black metric tiles (mock: white cards), Tahoma,
// Thai eyebrows letter-spaced, disabled buttons at half opacity, a misaligned order toolbar, and audit rows several
// lines tall from raw UUIDs and raw JSON.
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

const M = '.xc-devices .xc-shell-main';

describe('devices look', () => {
  it('DV-1 the page uses the theme font, not Tahoma', () => {
    const d = decls(readHere('devices.css'), '.xc-devices');
    expect(d?.get('font')).toBe('15px/1.65 var(--xc-font-sans)');
    expect(stripped(readHere('devices.css'))).not.toMatch(/Tahoma/);
  });

  it('DV-2 a metric is a white card with a muted label', () => {
    const css = readHere('devices.css');
    const d = decls(css, `${M} .metric`);
    expect(d?.get('background')).toBe('var(--xc-surface)');
    expect(d?.get('color')).toBe('var(--xc-ink)');
    expect(d?.get('border')).toBe('1px solid var(--xc-line)');
    expect(d?.get('border-radius')).toBe('var(--xc-radius-xl)');
    expect(d?.get('box-shadow')).toBe('var(--xc-shadow-sm)');
    expect(decls(css, `${M} .metric > span:first-child`)?.get('color')).toBe('var(--xc-muted)');
    expect(stripped(css)).not.toMatch(/var\(--xc-gray-900\)/);
  });

  it('DV-3 Thai eyebrows have no letter-spacing, in and out of cards', () => {
    const css = readHere('devices.css');
    expect(decls(css, `${M} .eyebrow:lang(th)`)?.get('letter-spacing')).toBe('0');
    expect(decls(css, `${M} .card .eyebrow:lang(th)`)?.get('letter-spacing')).toBe('0');
  });

  it('DV-4 a disabled button is solid grey, not half opacity', () => {
    const d = decls(readHere('devices.css'), `${M} button:disabled`);
    expect(d?.has('opacity')).toBe(false);
    expect(d?.get('background')).toBe('var(--xc-gray-100)');
    expect(d?.get('border-color')).toBe('var(--xc-line)');
    expect(d?.get('color')).toBe('var(--xc-gray-600)');
    expect(d?.get('cursor')).toBe('not-allowed');
  });

  it('DV-5 the order toolbar lines up: neither field group adds its own margin inside the grid', () => {
    const d = decls(readHere('devices.css'), `${M} .request-grid .reason-group`);
    expect(d?.get('margin')).toBe('0');
  });

  it('DV-6 ids in both tables are one line with the whole id in the title', () => {
    const css = readHere('devices.css');
    const d = decls(css, `${M} .dev-id`);
    expect(d?.get('max-width')).toBe('12ch');
    expect(d?.get('text-overflow')).toBe('ellipsis');
    expect(d?.get('white-space')).toBe('nowrap');
    const tsx = readHere('DevicesScreen.tsx');
    expect(tsx).toContain('<small class="dev-id" title={j.id}>{j.id}</small>');
    expect(tsx).toContain("<small class=\"dev-id\" title={j.ref || undefined}>{j.ref || t('noRef')}</small>");
    expect(tsx).toContain('<span class="dev-id" title={j.actor}>{j.actor}</span>');
    expect(tsx).toContain('<span class="dev-id" title={ev.actor}>{ev.actor}</span>');
    expect(tsx).toContain("<small class=\"dev-id\" title={ev.ref || undefined}>{ev.ref || '—'}</small>");
  });

  it('DV-7 event detail is one line above 640px, with the whole text in the title', () => {
    const wide = media(readHere('devices.css'), 'min-width: 641px');
    const d = decls(wide, `${M} .events-table td code`);
    expect(d?.get('max-width')).toBe('360px');
    expect(d?.get('white-space')).toBe('nowrap');
    expect(d?.get('text-overflow')).toBe('ellipsis');
    expect(readHere('DevicesScreen.tsx')).toContain('<code title={ev.detail}>{ev.detail}</code>');
  });
});
