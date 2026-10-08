// V1-login (#396 R1, design W6 and W10): the login screen's own text table.
// Its Thai is v2's, byte for byte (xchange-v2-source@4d0bdbb:src/devices-public/login.html:1:
// `<title>` / `<h1 id="title">` `เข้าสู่ระบบพนักงาน`, label `ชื่อผู้ใช้`, label `รหัสผ่าน`,
// button `เข้าสู่ระบบ`). The screen's other tests assert its text through this table.
// Written before the implementation by the lane controller. Do not edit, delete or weaken.
//
// The table this test pins, in `web/src/screens/login/i18n.ts` (W10 point 1):
//
//   export const th = { title, username, password, submit, ...any further keys } as const;
//   export const en: Record<keyof typeof th, string>;
//
// The core collects it under the namespace `login.*`.
import { describe, expect, it } from 'vitest';
import { screenLookup } from '../../shared/i18n';
import { en, th } from './i18n';

const THAI = /[฀-๿]/;

describe('login i18n table (#396)', () => {
  it("th carries v2's Thai verbatim (W6)", () => {
    expect(th.title).toBe('เข้าสู่ระบบพนักงาน');
    expect(th.username).toBe('ชื่อผู้ใช้');
    expect(th.password).toBe('รหัสผ่าน');
    expect(th.submit).toBe('เข้าสู่ระบบ');
  });

  it('en has exactly the keys of th, each an English, non-empty text', () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [key, text] of Object.entries(en)) {
      expect(typeof text, key).toBe('string');
      expect(text.trim().length, key).toBeGreaterThan(0);
      expect(THAI.test(text), key).toBe(false);
    }
  });

  it('is collected by the shared core under login.*', () => {
    expect(screenLookup.th['login.title']).toBe(th.title);
    expect(screenLookup.en['login.title']).toBe(en.title);
    expect(screenLookup.th['login.submit']).toBe(th.submit);
  });
});
