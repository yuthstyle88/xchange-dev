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

