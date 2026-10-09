// #763 (login follow-up of #762): the keys the invite step's "create one" and the four claim refusals add to
// the login screen's own text table (`web/src/screens/login/i18n.ts`). #762's i18n.test.ts stays as it is.
// Written before the implementation by the ticket author (claude/b060e9). Do not edit, delete or weaken.
//
// Keys this test pins (th and en both; en is non-Thai):
//   createAccount, phone, email                       — the toggle and the two new fields
//   err_inviteRefused_<reason>  for reason in inviteInvalid, alreadyLinked, userNotFound, userDisabled
//   err_<code> for the register refusals Identity answers through the BFF; the Thai text of these three is the BFF's
//   own sentence, byte for byte (api/crates/xc-auth/src/domain/identity/identity_client.rs @ abb7ea1):
//     phoneRequired :96 · invalidPhone :97 · weakPassword :98
//   err_alreadyRegistered is the screen's OWN sentence, not the BFF's (lane controller's ruling on #763,
//   2026-10-10): "this phone already has an account — sign in, then claim the invite". It must differ from the
//   BFF's :95 sentence, which only says "sign in".
import { describe, expect, it } from 'vitest';
import * as table from './i18n';

const TH = table.th as unknown as Record<string, string>;
const EN = table.en as unknown as Record<string, string>;
const THAI = /[฀-๿]/;

const REASONS = ['inviteInvalid', 'alreadyLinked', 'userNotFound', 'userDisabled'] as const;

const REGISTER_TH: Record<string, string> = {
  err_phoneRequired: 'กรุณากรอกเบอร์โทรศัพท์',
  err_invalidPhone: 'เบอร์โทรศัพท์ไม่ถูกต้อง',
  err_weakPassword: 'รหัสผ่านไม่ผ่านเงื่อนไขความปลอดภัย',
};

const NEW_KEYS = [
  'createAccount',
  'phone',
  'email',
  ...REASONS.map((r) => `err_inviteRefused_${r}`),
  'err_alreadyRegistered',
  ...Object.keys(REGISTER_TH),
];

/** The BFF's sentence for alreadyRegistered (identity_client.rs:95), which the screen must not reuse. */
const BFF_ALREADY_REGISTERED = 'มีบัญชีนี้อยู่แล้ว กรุณาเข้าสู่ระบบ';

describe('login i18n — register and refusal reasons (#763)', () => {
  it.each(NEW_KEYS)('%s is a Thai text in th and a non-Thai text in en', (key) => {
    expect(TH[key], `th.${key}`).toBeTypeOf('string');
    expect(TH[key]!.trim().length, `th.${key}`).toBeGreaterThan(0);
    expect(THAI.test(TH[key]!), `th.${key} is Thai`).toBe(true);
    expect(EN[key], `en.${key}`).toBeTypeOf('string');
    expect(EN[key]!.trim().length, `en.${key}`).toBeGreaterThan(0);
    expect(THAI.test(EN[key]!), `en.${key} is not Thai`).toBe(false);
  });

  it.each(Object.entries(REGISTER_TH))('th.%s is the BFF sentence for that code', (key, text) => {
    expect(TH[key]).toBe(text);
  });

  it('err_alreadyRegistered is the screen\'s own sentence, not the BFF\'s, and not any other error sentence', () => {
    expect(TH.err_alreadyRegistered, 'th.err_alreadyRegistered').toBeTypeOf('string');
    expect(EN.err_alreadyRegistered, 'en.err_alreadyRegistered').toBeTypeOf('string');
    expect(TH.err_alreadyRegistered).not.toBe(BFF_ALREADY_REGISTERED);
    for (const [name, t] of [
      ['th', TH],
      ['en', EN],
    ] as Array<[string, Record<string, string>]>) {
      const mine = t.err_alreadyRegistered;
      for (const [key, text] of Object.entries(t)) {
        if (key !== 'err_alreadyRegistered' && key.startsWith('err')) {
          expect(text, `${name}.${key} differs from ${name}.err_alreadyRegistered`).not.toBe(mine);
        }
      }
    }
  });

  it('the four reason sentences and the generic inviteRefused sentence all differ, in both languages', () => {
    const keys = ['err_inviteRefused', ...REASONS.map((r) => `err_inviteRefused_${r}`)];
    for (const [name, t] of [
      ['th', TH],
      ['en', EN],
    ] as Array<[string, Record<string, string>]>) {
      const texts = keys.map((k) => t[k]);
      expect(new Set(texts).size, `${name}: ${JSON.stringify(texts)}`).toBe(keys.length);
    }
  });
});
