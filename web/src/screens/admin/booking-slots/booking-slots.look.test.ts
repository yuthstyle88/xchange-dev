// OL-6 B admin-booking-slots (audit #564 issuecomment-6702970101, shot 047): sections are cards, fields sit in a grid,
// slot rows are one aligned line with unbroken Thai buttons.
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

describe('booking-slots look', () => {
  it('BL-1 every section is the shared card with 20px padding', () => {
    const css = readHere('booking-slots.css');
    expect(decls(css, '.xc-admin .xc-content .booking-slots-section')?.get('padding')).toBe('20px');
    const tsx = readHere('BookingSlots.tsx');
    expect(tsx).toContain("import '../../../shared/ui/card.css';");
    expect(tsx.match(/<section class="booking-slots-section card">/g)).toHaveLength(4);
    expect(tsx).not.toContain('<section class="booking-slots-section">');
  });

  it('BL-2 rule and policy fields share a responsive grid; both saves are primary', () => {
    const css = readHere('booking-slots.css');
    const d = decls(css, '.bs-fields');
    expect(d?.get('display')).toBe('grid');
    expect(d?.get('grid-template-columns')).toBe('repeat(auto-fit, minmax(200px, 1fr))');
    const tsx = readHere('BookingSlots.tsx');
    expect(tsx.match(/<div class="bs-fields">/g)).toHaveLength(2);
    expect(tsx).toContain('class="primary" disabled={writing()} onClick={handleSaveRule}');
    expect(tsx).toContain('class="primary" disabled={writing()} onClick={handleSavePolicy}');
  });

  it('BL-3 the closures form is one aligned row; its counter is small and muted', () => {
    const css = readHere('booking-slots.css');
    const f = decls(css, '.xc-admin .xc-content .closures-form');
    expect(f?.get('display')).toBe('grid');
    expect(f?.get('align-items')).toBe('start');
    expect(decls(css, '.xc-admin .xc-content .closures-form > button')?.get('margin-top')).toBe('28px');
    // Restated by the sub-controller for OL-8 reuse-field-count: the counter uses the shared .xc-field-count.
    expect(decls(readHere('../../../shared/ui/field-count.css'), '.xc-field-count')?.get('color')).toBe('var(--xc-muted)');
    expect(readHere('BookingSlots.tsx')).toContain('<div class="xc-field-count">{newReason().length} / 200</div>');
  });

  it('BL-4 a slot row is a grid line; the date shows once in the day summary', () => {
    const css = readHere('booking-slots.css');
    expect(decls(css, '.xc-admin .xc-content .slot-item')?.get('display')).toBe('grid');
    expect(decls(css, '.slot-item .slot-date')?.get('display')).toBe('none');
    expect(decls(css, '.xc-admin .xc-content.xc-content .slot-item button')?.get('white-space')).toBe('nowrap');
    expect(
      decls(css, '.xc-admin .xc-content.xc-content .slot-item .slot-capacity-control input[name="slotCapacity"]')?.get('width'),
    ).toBe('88px');
  });

  it('BL-5 status is a pill; CLOSED is grey', () => {
    const css = readHere('booking-slots.css');
    expect(decls(css, '.slot-item .slot-status')?.get('border-radius')).toBe('var(--xc-radius-pill)');
    expect(decls(css, '.slot-item .slot-status[data-status="CLOSED"]')?.get('color')).toBe('var(--xc-gray-600)');
  });

  it('BL-6 a day summary is a 44px row with the count on the right', () => {
    const d = decls(readHere('booking-slots.css'), '.bs-day > summary');
    expect(d?.get('min-height')).toBe('44px');
    expect(d?.get('justify-content')).toBe('space-between');
  });

  it('BL-7 at 800px or less the form and slot rows stack', () => {
    const m = media(readHere('booking-slots.css'), 'max-width: 800px');
    expect(decls(m, '.xc-admin .xc-content .slot-item')?.get('grid-template-columns')).toBe('1fr');
    expect(decls(m, '.xc-admin .xc-content .closures-form')?.get('grid-template-columns')).toBe('1fr');
  });
});
