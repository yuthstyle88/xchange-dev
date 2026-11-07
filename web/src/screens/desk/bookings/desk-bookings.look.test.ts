// OL-6 wave B desk-bookings (audit #564 issuecomment-6702970101, shot 006): the toolbar is a grey box, not a card; the
// QR field and buttons use a 4px radius and beat the theme's 44px fields; the QR field has no label; "Refresh" is
// hard-coded English; the empty state is bare text. Written before the fix by claude/bbe9ce8b. Do not edit, delete or
// weaken. DeskBookings.lang.test.tsx pins the QR placeholder to `scanQr`, so the placeholder stays.
import { describe, expect, it } from 'vitest';
import { en, th } from './i18n';

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

describe('desk-bookings look', () => {
  it('DB-1 the toolbar is the shared card, with no grey box of its own', async () => {
    const src = await readHere('DeskBookingsScreen.tsx');
    expect(src).toMatch(/import '\.\.\/\.\.\/\.\.\/shared\/ui\/card\.css';/);
    expect(src).toContain('class="xc-bookings-toolbar card"');
    const d = decls(await readHere('desk-bookings.css'), '.xc-bookings-toolbar');
    for (const prop of ['background-color', 'background', 'border', 'border-radius', 'padding']) {
      expect(d?.has(prop), `.xc-bookings-toolbar must not set ${prop}`).toBe(false);
    }
  });

  it('DB-2 the QR input leaves height, radius and border to the theme', async () => {
    const d = decls(await readHere('desk-bookings.css'), '.xc-qr-input');
    expect(d?.get('min-width')).toBe('240px');
    for (const prop of ['padding', 'border', 'border-radius', 'font-size']) {
      expect(d?.has(prop), `.xc-qr-input must not set ${prop}`).toBe(false);
    }
  });

  it('DB-3 the dialog fields leave height, radius and border to the theme too', async () => {
    const d = decls(await readHere('desk-bookings.css'), '.xc-form-group input');
    expect(d, '.xc-form-group input rule removed').toBeUndefined();
  });

  it.each(['.xc-btn-primary', '.xc-btn-secondary'])('DB-4 %s has the 44px button shape', async (sel) => {
    const d = decls(await readHere('desk-bookings.css'), sel);
    expect(d?.get('min-height')).toBe('44px');
    expect(d?.get('padding')).toBe('10px 18px');
    expect(d?.get('border-radius')).toBe('var(--xc-radius-md)');
    expect(d?.get('font-weight')).toBe('600');
  });

  it('DB-5 secondary buttons use the brand text colour; primary keeps a brand border', async () => {
    const css = await readHere('desk-bookings.css');
    expect(decls(css, '.xc-btn-secondary')?.get('color')).toBe('var(--xc-brand)');
    expect(decls(css, '.xc-btn-primary')?.get('border')).toBe('1px solid var(--xc-brand)');
  });

  it('DB-6 the QR field has a visible label wrapping the input', async () => {
    const src = await readHere('DeskBookingsScreen.tsx');
    expect(src).toMatch(/<label class="xc-qr-field">\s*<span>\{t\('qrLabel'\)\}<\/span>\s*<input\s+name="qrToken"/);
    expect(th.qrLabel).toBe('รหัส QR ของการจอง');
    expect(en.qrLabel).toBe('Booking QR code');
    const d = decls(await readHere('desk-bookings.css'), '.xc-qr-field');
    expect(d?.get('display')).toBe('grid');
    expect(d?.get('gap')).toBe('4px');
  });

  it('DB-7 the toolbar rows line up on the field, not on its label', async () => {
    const css = await readHere('desk-bookings.css');
    expect(decls(css, '.xc-bookings-toolbar')?.get('align-items')).toBe('flex-end');
    expect(decls(css, '.xc-qr-search')?.get('align-items')).toBe('flex-end');
    expect(decls(css, '.xc-qr-search')?.get('flex-wrap')).toBe('wrap');
  });

  it('DB-8 Refresh is translated', async () => {
    const src = await readHere('DeskBookingsScreen.tsx');
    expect(src).not.toMatch(/>\s*Refresh\s*</);
    expect(src).toContain("{t('refresh')}");
    expect(th.refresh).toBe('รีเฟรช');
    expect(en.refresh).toBe('Refresh');
  });

  it('DB-9 the empty state is a card', async () => {
    const src = await readHere('DeskBookingsScreen.tsx');
    expect(src).toContain('class="xc-empty-state card"');
    const d = decls(await readHere('desk-bookings.css'), '.xc-empty-state');
    expect(d?.get('text-align')).toBe('center');
    expect(d?.get('color')).toBe('var(--xc-muted)');
  });
});
