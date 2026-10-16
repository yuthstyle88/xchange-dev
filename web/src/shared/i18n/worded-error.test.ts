// WE-1: one shared way to word a failed call (docs/design/web-worded-error.md WE1–WE3).
// Written before the implementation by the lane controller (claude/e92db6). Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { ApiError } from '../api/client';
import { setLang } from './index';
import { en } from './en';
import { th } from './th';
import { zh } from './zh';
import { wordedError } from './worded-error';

const refusal = (status: number, code: string, message: string) => new ApiError({ status, code, message });

afterEach(() => setLang('th'));

describe('WE2 (a): a server failure always reads as "server error"', () => {
  it('a 500 serverError shows the shared serverError text in TH and EN', () => {
    const err = refusal(500, 'serverError', 'internal: pool timed out');
    expect(wordedError(err)).toBe(th['error.serverError']);
    setLang('en');
    expect(wordedError(err)).toBe(en['error.serverError']);
  });

  it('any 5xx wins over its code and its message, even when the code has a shared key', () => {
    expect(wordedError(refusal(503, 'notFound', 'upstream down'))).toBe(th['error.serverError']);
    expect(wordedError(refusal(502, 'someCode', 'bad gateway'))).toBe(th['error.serverError']);
  });

  it('an httpError (no JSON body) shows serverError, never its raw body', () => {
    const err = refusal(400, 'httpError', '<html>Bad Request</html>');
    expect(wordedError(err)).toBe(th['error.serverError']);
    expect(wordedError(err)).not.toContain('<html>');
  });
});

describe('WE2 (b): a refusal follows W10 (4) unchanged', () => {
  it('a code with a shared key shows that key, not the server message', () => {
    const err = refusal(403, 'roleNotAllowedHere', 'RAW wire text');
    expect(wordedError(err)).toBe(th['error.roleNotAllowedHere']);
    setLang('en');
    expect(wordedError(err)).toBe(en['error.roleNotAllowedHere']);
  });

  it('a known code whose server message equals the Thai key text still shows the key text', () => {
    const err = refusal(403, 'roleNotAllowedHere', th['error.roleNotAllowedHere']);
    expect(wordedError(err)).toBe(th['error.roleNotAllowedHere']);
    setLang('en');
    expect(wordedError(err)).toBe(en['error.roleNotAllowedHere']);
  });

  it('a code with NO shared key shows the server sentence, in every language (W10 (4))', () => {
    const sentence = 'ลบกลุ่มนี้ไม่ได้ เพราะประกาศเรทปัจจุบันยังใช้อยู่';
    const err = refusal(409, 'groupPricedByPublication', sentence);
    expect(wordedError(err)).toBe(sentence);
    setLang('en');
    expect(wordedError(err)).toBe(sentence);
  });

  it('a code with no key and an empty message shows serverError, never the bare code', () => {
    const err = refusal(400, 'groupPricedByPublication', '');
    expect(wordedError(err)).toBe(th['error.serverError']);
    expect(wordedError(err)).not.toContain('groupPricedByPublication');
  });

  it('a code with no key whose message is just the code shows serverError', () => {
    const err = refusal(400, 'groupPricedByPublication', 'groupPricedByPublication');
    expect(wordedError(err)).toBe(th['error.serverError']);
  });
});

describe('WE2 (c) + WE3: the call never reached the server', () => {
  it('a fetch TypeError shows error.network in TH and EN, never the JS message', () => {
    const err = new TypeError('Failed to fetch');
    expect(wordedError(err)).toBe(th['error.network']);
    expect(wordedError(err)).not.toContain('Failed to fetch');
    setLang('en');
    expect(wordedError(err)).toBe(en['error.network']);
  });

  it('a plain Error, a thrown string, null and undefined all show error.network', () => {
    for (const thrown of [new Error('boom'), 'boom', null, undefined, { code: 'x' }]) {
      expect(wordedError(thrown)).toBe(th['error.network']);
    }
  });

  it('error.network has the sentences WE3 fixes, in all three tables', () => {
    expect(th['error.network']).toBe('เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่');
    expect(en['error.network']).toBe('Cannot reach the server. Please try again.');
    expect(zh['error.network']).toBe('无法连接服务器，请重试。');
  });
});

import { isServerFailure } from './worded-error';

describe('isServerFailure (#1862 §9)', () => {
  it('row 1: ApiError{status:500, code:"dbTimeout"} -> true', () => {
    expect(isServerFailure(refusal(500, 'dbTimeout', 'db timeout'))).toBe(true);
  });

  it('row 2: ApiError{status:503, code:"roleNotAllowedHere"} -> true (status wins over a keyed code)', () => {
    const err = refusal(503, 'roleNotAllowedHere', 'upstream unavailable');
    expect(isServerFailure(err)).toBe(true);
    expect(wordedError(err)).toBe(th['error.serverError']);
  });

  it('row 3: ApiError{status:400, code:"serverError"} -> true', () => {
    expect(isServerFailure(refusal(400, 'serverError', 'badserver'))).toBe(true);
  });

  it('row 4: ApiError{status:400, code:"httpError"} -> true', () => {
    expect(isServerFailure(refusal(400, 'httpError', '<html>error</html>'))).toBe(true);
  });

  it('row 5: ApiError{status:403, code:"roleNotAllowedHere"} -> false', () => {
    expect(isServerFailure(refusal(403, 'roleNotAllowedHere', 'forbidden'))).toBe(false);
  });

  it('row 6: ApiError{status:499, code:"x"} -> false', () => {
    expect(isServerFailure(refusal(499, 'x', 'client closed'))).toBe(false);
  });

  it('row 7: new TypeError("Failed to fetch") -> false', () => {
    expect(isServerFailure(new TypeError('Failed to fetch'))).toBe(false);
    expect(isServerFailure(Object.assign(new TypeError('Failed to fetch'), { status: 500 }))).toBe(false);
  });

  it('row 8: undefined / a string -> false', () => {
    expect(isServerFailure(undefined)).toBe(false);
    expect(isServerFailure('serverError')).toBe(false);
    expect(isServerFailure({ status: 500, code: 'serverError' })).toBe(false);
  });
});

import { setAudience } from './index';

const STALE_TH = 'ข้อมูลมีการเปลี่ยนแปลง กรุณาโหลดหน้าใหม่แล้วลองอีกครั้ง';
const STALE_EN = 'This data has changed; reload and try again';
const STALE_ZH = '数据已更改，请刷新后重试';

const CONFLICT_TH = 'คำขอนี้ถูกส่งไปแล้วด้วยข้อมูลอื่น กรุณาโหลดหน้าใหม่แล้วลองอีกครั้ง';
const CONFLICT_EN = 'This request was already sent with other data; reload and try again';
const CONFLICT_ZH = '该请求已使用其他数据发送，请刷新后重试';

describe('shared staleVersion and idempotencyConflict keys (#2353)', () => {
  afterEach(() => {
    setAudience('staff');
    setLang('th');
  });

  it('row 1: ApiError 409 staleVersion with message "stale version", lang th / en -> table text, never raw message', () => {
    const err = refusal(409, 'staleVersion', 'stale version');
    expect((th as Record<string, string>)['error.staleVersion']).toBe(STALE_TH);
    expect((en as Record<string, string>)['error.staleVersion']).toBe(STALE_EN);

    expect(wordedError(err)).toBe(STALE_TH);
    expect(wordedError(err)).not.toContain('stale version');

    setLang('en');
    expect(wordedError(err)).toBe(STALE_EN);
    expect(wordedError(err)).not.toContain('stale version');
  });

  it('row 2: staleVersion with "staleVersion: transaction X does not exist" and with "STALE_VERSION" -> table text', () => {
    const errTx = refusal(409, 'staleVersion', 'staleVersion: transaction X does not exist');
    const errToken = refusal(409, 'staleVersion', 'STALE_VERSION');

    expect(wordedError(errTx)).toBe(STALE_TH);
    expect(wordedError(errToken)).toBe(STALE_TH);

    setLang('en');
    expect(wordedError(errTx)).toBe(STALE_EN);
    expect(wordedError(errToken)).toBe(STALE_EN);

    setAudience('customer');
    setLang('zh');
    expect(wordedError(errTx)).toBe(STALE_ZH);
    expect(wordedError(errToken)).toBe(STALE_ZH);
  });

  it('row 3: idempotencyConflict with "IDEMPOTENCY_CONFLICT" and "idempotency conflict", th / en / zh (customer page) -> table text in that language', () => {
    const errUpper = refusal(409, 'idempotencyConflict', 'IDEMPOTENCY_CONFLICT');
    const errLower = refusal(409, 'idempotencyConflict', 'idempotency conflict');

    expect((th as Record<string, string>)['error.idempotencyConflict']).toBe(CONFLICT_TH);
    expect((en as Record<string, string>)['error.idempotencyConflict']).toBe(CONFLICT_EN);
    expect((zh as Record<string, string>)['error.idempotencyConflict']).toBe(CONFLICT_ZH);

    expect(wordedError(errUpper)).toBe(CONFLICT_TH);
    expect(wordedError(errLower)).toBe(CONFLICT_TH);

    setLang('en');
    expect(wordedError(errUpper)).toBe(CONFLICT_EN);
    expect(wordedError(errLower)).toBe(CONFLICT_EN);

    setAudience('customer');
    setLang('zh');
    expect(wordedError(errUpper)).toBe(CONFLICT_ZH);
    expect(wordedError(errLower)).toBe(CONFLICT_ZH);
  });

  it('row 4: 5xx carrying either code -> error.serverError (kept)', () => {
    expect(wordedError(refusal(503, 'staleVersion', 'stale version'))).toBe(th['error.serverError']);
    expect(wordedError(refusal(500, 'idempotencyConflict', 'IDEMPOTENCY_CONFLICT'))).toBe(
      th['error.serverError'],
    );
    expect(wordedError(refusal(409, 'staleVersion', 'stale version'))).toBe(STALE_TH);
    expect(wordedError(refusal(409, 'idempotencyConflict', 'IDEMPOTENCY_CONFLICT'))).toBe(
      CONFLICT_TH,
    );
  });

  it('row 5: A message equal to the code (staleVersion/staleVersion) -> the table text, no longer serverError', () => {
    const errStale = refusal(409, 'staleVersion', 'staleVersion');
    const errConflict = refusal(409, 'idempotencyConflict', 'idempotencyConflict');

    expect(wordedError(errStale)).toBe(STALE_TH);
    expect(wordedError(errStale)).not.toBe(th['error.serverError']);
    expect(wordedError(errConflict)).toBe(CONFLICT_TH);
    expect(wordedError(errConflict)).not.toBe(th['error.serverError']);
  });

  it("row 6: A code with no shared key -> the server's sentence (kept, worded-error.test.ts:50)", () => {
    const sentence = 'ลบกลุ่มนี้ไม่ได้ เพราะประกาศเรทปัจจุบันยังใช้อยู่';
    expect(wordedError(refusal(409, 'groupPricedByPublication', sentence))).toBe(sentence);
    expect(wordedError(refusal(409, 'staleVersion', sentence))).toBe(STALE_TH);
    expect(wordedError(refusal(409, 'idempotencyConflict', sentence))).toBe(CONFLICT_TH);
  });
});

