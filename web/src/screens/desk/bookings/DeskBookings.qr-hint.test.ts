// OL-7 row 11 (VERIFY.md on main 4a577dac, shot 006): on /desk/bookings the QR field's placeholder read "สแกน QR", the
// same words as the lookup button beside it, so the field gave no hint of what to put in it. The field gets its own
// hint text (`qrPlaceholder`); the button keeps `scanQr`. DeskBookings.lang.test.tsx (restated) checks the rendered
// placeholder in both languages. Written before the fix by claude/studio-claude-17. Do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';
import { en, th } from './i18n';

describe('desk bookings QR field hint', () => {
  it('QH-1 the field hint is its own text in Thai and English', () => {
    expect((th as Record<string, string>).qrPlaceholder).toBe('สแกนหรือพิมพ์รหัส QR');
    expect((en as Record<string, string>).qrPlaceholder).toBe('Scan or type the QR code');
  });

  it('QH-2 the hint never repeats the button label or the field label', () => {
    for (const d of [th, en] as Array<Record<string, string>>) {
      expect(d.qrPlaceholder).toBeTruthy();
      expect(d.qrPlaceholder).not.toBe(d.scanQr);
      expect(d.qrPlaceholder).not.toBe(d.qrLabel);
    }
  });
});
