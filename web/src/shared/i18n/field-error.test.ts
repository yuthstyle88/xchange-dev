// T0 (#4897): one shared wording for a refused field, `fieldError(err, labels)`
// (docs/design/web-input-validation.md §2; ticket §3 rows F1–F7).
// Written before the implementation by the XC FE lane controller (claude/e8020026). Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { ApiError } from '../api/client';
import { fill, setAudience, setLang } from './index';
import { en } from './en';
import { th } from './th';
import { zh } from './zh';
import { wordedError } from './worded-error';
import { fieldError } from './field-error';

/** The tables read by key name, so a key the implementation has not added yet fails at runtime, not in tsc. */
const TABLES = {
  th: th as Record<string, string>,
  en: en as Record<string, string>,
  zh: zh as Record<string, string>,
};
type Lang = keyof typeof TABLES;

/** The text of a shared key; fails the test when the key is missing or empty. */
function text(lang: Lang, key: string): string {
  const value = TABLES[lang][key];
  expect(typeof value, `${lang} ${key}`).toBe('string');
  expect(value.trim(), `${lang} ${key}`).not.toBe('');
  return value;
}

function use(lang: Lang): void {
  setAudience(lang === 'zh' ? 'customer' : 'staff'); // zh is a customer-page language only (languages.ts SPOKEN)
  setLang(lang);
}

/** The server's v3 input refusal: `400 {code:"validationFailed", field, message:"ข้อมูลไม่ถูกต้อง"}` (org_view.rs:94). */
const SERVER_THAI = 'ข้อมูลไม่ถูกต้อง';
const refusal = (init: { status?: number; code: string; field?: string; message?: string }) =>
  new ApiError({ status: init.status ?? 400, code: init.code, field: init.field, message: init.message ?? SERVER_THAI });

const CODES = [
  'invalidPhone',
  'phoneRequired',
  'invalidCode',
  'codeExpired',
  'tooManyAttempts',
  'invalidCredentials',
  'nameTooLong',
  'quoteRefused',
  'expiresAtNotInFuture',
] as const;

const INPUT_KEYS = ['input.invalid', 'input.invalidNoField', ...CODES.map((c) => `input.code.${c}`)];
const KIOSK_KEYS = ['kioskCancel.noteRequired', 'kioskCancel.noteTooLong'];
const THAI_SCRIPT = /[฀-๿]/;

function placeholders(s: string): string[] {
  return [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort();
}

afterEach(() => {
  setAudience('staff');
  setLang('th');
});

describe('F1: validationFailed at a field the screen labels → that field, named in the sentence', () => {
  it('en: {field:"quantity", message:"Amount is not valid"}', () => {
    use('en');
    const r = fieldError(refusal({ code: 'validationFailed', field: 'quantity' }), { quantity: 'Amount' });
    expect(r).toStrictEqual({ field: 'quantity', message: 'Amount is not valid' });
  });

  it('th: "{field} ไม่ถูกต้อง" with the Thai label, never the bare server text', () => {
    use('th');
    const r = fieldError(refusal({ code: 'validationFailed', field: 'quantity' }), { quantity: 'จำนวน' });
    expect(r).toStrictEqual({ field: 'quantity', message: 'จำนวน ไม่ถูกต้อง' });
  });

  it('zh: "{field}无效"', () => {
    use('zh');
    const r = fieldError(refusal({ code: 'validationFailed', field: 'quantity' }), { quantity: '数量' });
    expect(r).toStrictEqual({ field: 'quantity', message: '数量无效' });
  });
});

describe('F2: validationFailed at a field the screen does not label → input.invalidNoField, no field', () => {
  for (const lang of ['th', 'en', 'zh'] as const) {
    it(`${lang}: field "other" with labels {quantity} reads input.invalidNoField and carries no field`, () => {
      use(lang);
      const r = fieldError(refusal({ code: 'validationFailed', field: 'other' }), { quantity: 'Amount' });
      expect(r).toStrictEqual({ message: text(lang, 'input.invalidNoField') });
      expect(r.field).toBeUndefined();
    });
  }

  it('en: no field at all reads input.invalidNoField, never the Thai server message', () => {
    use('en');
    const r = fieldError(refusal({ code: 'validationFailed' }), { quantity: 'Amount' });
    expect(r).toStrictEqual({ message: text('en', 'input.invalidNoField') });
    expect(r.message).not.toBe(SERVER_THAI);
    expect(r.message).not.toMatch(THAI_SCRIPT);
  });

  it('a field name inherited from Object.prototype is not "a key of labels"', () => {
    use('en');
    const r = fieldError(refusal({ code: 'validationFailed', field: 'constructor' }), {});
    expect(r).toStrictEqual({ message: text('en', 'input.invalidNoField') });
  });
});

describe('F3: a code with a shared input.code.<code> key → that text, at the field when labelled', () => {
  for (const lang of ['th', 'en'] as const) {
    it(`${lang}: invalidPhone at "phone" → {field:"phone", message: input.code.invalidPhone}`, () => {
      use(lang);
      const r = fieldError(refusal({ code: 'invalidPhone', field: 'phone', message: 'raw server text' }), { phone: 'Phone' });
      expect(r).toStrictEqual({ field: 'phone', message: text(lang, 'input.code.invalidPhone') });
      expect(r.message).not.toBe('raw server text');
    });
  }

  it('zh: invalidPhone reads the zh input.code.invalidPhone text', () => {
    use('zh');
    const r = fieldError(refusal({ code: 'invalidPhone', field: 'phone' }), { phone: '电话' });
    expect(r).toStrictEqual({ field: 'phone', message: text('zh', 'input.code.invalidPhone') });
  });

  it('the keyed text without a labelled field carries no field', () => {
    use('en');
    const r = fieldError(refusal({ code: 'invalidPhone', field: 'phone' }), {});
    expect(r).toStrictEqual({ message: text('en', 'input.code.invalidPhone') });
    expect(r.field).toBeUndefined();
  });

  it('every one of the nine codes reads its own input.code.<code> key, in th and en', () => {
    for (const lang of ['th', 'en'] as const) {
      use(lang);
      for (const code of CODES) {
        const r = fieldError(refusal({ code, field: 'x', message: 'raw server text' }), { x: 'X' });
        expect(r, `${lang} ${code}`).toStrictEqual({ field: 'x', message: text(lang, `input.code.${code}`) });
      }
    }
  });
});

describe('F4: a server failure → error.serverError, no field', () => {
  it('a 500 reads error.serverError even when its code and field would otherwise name a labelled field', () => {
    use('en');
    const r = fieldError(refusal({ status: 500, code: 'validationFailed', field: 'quantity' }), { quantity: 'Amount' });
    expect(r).toStrictEqual({ message: en['error.serverError'] });
    expect(r.field).toBeUndefined();
  });

  it('a 503 with a keyed code, a serverError code and an httpError all read error.serverError (th)', () => {
    use('th');
    const labels = { phone: 'Phone' };
    for (const err of [
      refusal({ status: 503, code: 'invalidPhone', field: 'phone' }),
      refusal({ status: 400, code: 'serverError', field: 'phone' }),
      refusal({ status: 400, code: 'httpError', field: 'phone', message: '<html>Bad Request</html>' }),
    ]) {
      expect(fieldError(err, labels), err.code).toStrictEqual({ message: th['error.serverError'] });
    }
  });
});

describe('F5: the call never reached the server → error.network, no field', () => {
  it('a fetch TypeError reads error.network in th and en', () => {
    use('th');
    expect(fieldError(new TypeError('Failed to fetch'), { quantity: 'จำนวน' })).toStrictEqual({ message: th['error.network'] });
    use('en');
    expect(fieldError(new TypeError('Failed to fetch'), { quantity: 'Amount' })).toStrictEqual({ message: en['error.network'] });
  });

  it('a look-alike that is not an ApiError is never routed to a field', () => {
    use('en');
    const fake = { status: 400, code: 'validationFailed', field: 'quantity', message: SERVER_THAI };
    for (const thrown of [fake, new Error('boom'), 'boom', null, undefined]) {
      expect(fieldError(thrown, { quantity: 'Amount' })).toStrictEqual({ message: en['error.network'] });
    }
  });
});

describe('F6: any other refusal → exactly what wordedError gives today', () => {
  for (const lang of ['th', 'en'] as const) {
    it(`${lang}: ApiError{someCode, message:"x"} with labels {} → {message: wordedError(err)}`, () => {
      use(lang);
      const err = refusal({ status: 409, code: 'someCode', message: 'x' });
      const r = fieldError(err, {});
      expect(r).toStrictEqual({ message: wordedError(err) });
      expect(r.message).toBe('x');
    });
  }

  it('a code with a shared error.* key keeps that wording, and keeps a labelled field', () => {
    use('en');
    const err = refusal({ status: 403, code: 'roleNotAllowedHere', field: 'quantity', message: 'RAW wire text' });
    expect(fieldError(err, { quantity: 'Amount' })).toStrictEqual({
      field: 'quantity',
      message: en['error.roleNotAllowedHere'],
    });
  });

  it('an unlabelled field is dropped', () => {
    use('en');
    const err = refusal({ status: 409, code: 'someCode', field: 'other', message: 'x' });
    expect(fieldError(err, { quantity: 'Amount' })).toStrictEqual({ message: 'x' });
  });
});

describe('F7: th, en and zh hold the same input.* keys', () => {
  it('the three tables have exactly the same input.* keys, and they include every key the ticket names', () => {
    const inputKeys = (lang: Lang) => Object.keys(TABLES[lang]).filter((k) => k.startsWith('input.')).sort();
    expect(inputKeys('th')).toEqual([...INPUT_KEYS].sort());
    expect(inputKeys('en')).toEqual(inputKeys('th'));
    expect(inputKeys('zh')).toEqual(inputKeys('th'));
  });

  it('every new key is non-empty in all three tables and has the same placeholders', () => {
    for (const key of [...INPUT_KEYS, ...KIOSK_KEYS]) {
      const p = placeholders(text('th', key));
      expect(placeholders(text('en', key)), `en ${key}`).toEqual(p);
      expect(placeholders(text('zh', key)), `zh ${key}`).toEqual(p);
    }
    expect(placeholders(text('th', 'input.invalid'))).toEqual(['field']);
    expect(placeholders(text('th', 'kioskCancel.noteTooLong'))).toEqual(['max']);
    expect(placeholders(text('th', 'kioskCancel.noteRequired'))).toEqual([]);
  });

  it('th is Thai; en and zh are not', () => {
    for (const key of [...INPUT_KEYS, ...KIOSK_KEYS]) {
      expect(text('th', key), key).toMatch(THAI_SCRIPT);
      expect(text('en', key), key).not.toMatch(THAI_SCRIPT);
      expect(text('zh', key), key).not.toMatch(THAI_SCRIPT);
    }
  });

  it('input.invalid and input.invalidNoField read exactly as the ticket words them', () => {
    expect(fill(text('th', 'input.invalid'), { field: 'F' })).toBe('F ไม่ถูกต้อง');
    expect(fill(text('en', 'input.invalid'), { field: 'F' })).toBe('F is not valid');
    expect(fill(text('zh', 'input.invalid'), { field: 'F' })).toBe('F无效');
    expect(text('th', 'input.invalidNoField')).toBe('ข้อมูลไม่ถูกต้อง กรุณาตรวจสอบค่าที่กรอก');
    expect(text('en', 'input.invalidNoField')).toBe('Some input is not valid. Please check it.');
    expect(text('zh', 'input.invalidNoField')).toBe('部分输入无效，请检查。');
  });
});
