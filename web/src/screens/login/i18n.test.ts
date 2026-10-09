// OR-3 web follow-up (#762 §7 `i18n.ts` [required]; design W6 and W10): the login screen's own text table.
// The title, password and submit stay v2's Thai byte for byte
// (xchange-v2-source@4d0bdbb:src/devices-public/login.html:1, pinned since #396).
// The username label becomes the Identity identifier label: Identity signs in by e-mail, phone or username
// (108-platform-services@681bea2 crates/auth/src/application/login.rs:114-125), so v2's `ชื่อผู้ใช้`
// (the retired pin, this file's old :22) no longer tells a person what to type.
// Each `err_<code>` Thai text is the BFF's own sentence for that code, byte for byte (@ 8234e77):
//   identity_client.rs = api/crates/xc-auth/src/domain/identity/identity_client.rs
//   session_service.rs = api/crates/xc-auth/src/application/session/session_service.rs
//   user_service.rs    = api/crates/xc-auth/src/application/user/user_service.rs
// The screen's other tests assert its text through this table.
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken.
//
// The table this test pins, in `web/src/screens/login/i18n.ts`:
//
//   export const th = { title, identifier, password, submit, haveInvite, inviteCode,
//                       err_invalidCredentials, err_accountLocked, err_tooManyAttempts, err_identityDisabled,
//                       err_userDisabled, err_signIn, err_inviteRefused, err_identityUnavailable,
//                       err_invalidRequest, errUnknown, errNetwork, ...any further keys } as const;
//   export const en: Record<keyof typeof th, string>;
//
// The core collects it under the namespace `login.*`.
import { describe, expect, it } from 'vitest';
import { screenLookup } from '../../shared/i18n';
import * as table from './i18n';

const TH = table.th as unknown as Record<string, string>;
const EN = table.en as unknown as Record<string, string>;
const THAI = /[฀-๿]/;

/** The Thai this ticket pins, key by key, with where each sentence comes from. */
const PINNED_TH: Record<string, string> = {
  title: 'เข้าสู่ระบบพนักงาน', // v2 login.html:1
  password: 'รหัสผ่าน', // v2 login.html:1
  submit: 'เข้าสู่ระบบ', // v2 login.html:1
  identifier: 'อีเมล เบอร์โทรศัพท์ หรือชื่อผู้ใช้',
  haveInvite: 'มีรหัสเชิญ',
  inviteCode: 'รหัสเชิญ',
  err_invalidCredentials: 'ชื่อผู้ใช้หรือรหัสผ่านไม่ถูกต้อง', // identity_client.rs:84
  err_accountLocked: 'บัญชีถูกล็อกชั่วคราว กรุณาลองใหม่ภายหลัง', // identity_client.rs:85
  err_tooManyAttempts: 'ลองหลายครั้งเกินไป กรุณารอสักครู่แล้วลองใหม่', // identity_client.rs:86-90
  err_identityDisabled: 'บัญชีนี้ถูกระงับที่ระบบยืนยันตัวตน', // identity_client.rs:91
  err_userDisabled: 'บัญชีถูกระงับ', // session_service.rs:71
  err_signIn: 'บัญชีนี้ยังไม่ได้ผูกกับผู้ใช้ของ XCHANGE กรุณาใช้รหัสเชิญ', // session_service.rs:72-76 (UserNotBound)
  err_inviteRefused: 'รหัสเชิญไม่ถูกต้อง ถูกใช้งานแล้ว หรือหมดอายุแล้ว', // user_service.rs:141
  err_identityUnavailable: 'ระบบยืนยันตัวตนไม่พร้อมใช้งาน กรุณาลองใหม่ภายหลัง', // identity_client.rs:4
  err_invalidRequest: 'คำขอไม่ถูกต้อง', // auth.rs:202
  errUnknown: 'เกิดข้อผิดพลาด กรุณาลองใหม่',
  errNetwork: 'เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่',
};

describe('login i18n table (OR-3 web)', () => {
  it.each(Object.entries(PINNED_TH))('th.%s is pinned Thai', (key, text) => {
    expect(TH[key], `th.${key}`).toBe(text);
  });

  it('has no username key any more: the identifier label replaced it', () => {
    expect(Object.prototype.hasOwnProperty.call(TH, 'username')).toBe(false);
  });

  it('en has exactly the keys of th, each an English, non-empty text', () => {
    expect(Object.keys(EN).sort()).toEqual(Object.keys(TH).sort());
    for (const [key, text] of Object.entries(EN)) {
      expect(typeof text, key).toBe('string');
      expect(text.trim().length, key).toBeGreaterThan(0);
      expect(THAI.test(text), key).toBe(false);
    }
  });

  it('every error text differs from every other, in both languages', () => {
    const keys = Object.keys(PINNED_TH).filter((k) => k.startsWith('err'));
    for (const t of [TH, EN]) {
      const texts = keys.map((k) => t[k]);
      expect(new Set(texts).size).toBe(keys.length);
    }
  });

  it('is collected by the shared core under login.*', () => {
    expect(screenLookup.th['login.title']).toBe(TH.title);
    expect(screenLookup.en['login.title']).toBe(EN.title);
    expect(screenLookup.th['login.identifier']).toBe(PINNED_TH.identifier);
    expect(screenLookup.th['login.err_signIn']).toBe(PINNED_TH.err_signIn);
  });
});
