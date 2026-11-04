// F4 (#564 issuecomment-6624851184, owner "แก้ไขให้ถูกต้อง", 14:06 ICT 2026-11-04): a 409 stockInsufficient that
// carries `details.lines` (each {currency, place: DRAWER|SAFE|POSITION, shortBy}) is worded by the screen in the
// page's language, naming every short place. Without details it stays the shared text, as
// stock-insufficient.test.ts pins: the server's `message` is never shown raw.
// Written before the implementation by the ticket author claude/01TxaRJG. Do not edit, delete or weaken.
import { afterEach, expect, it } from 'vitest';
import { ApiError } from '../api/client';
import { setAudience, setLang } from './index';
import { th } from './th';
import { wordedError } from './worded-error';

afterEach(() => {
  setAudience('staff');
  setLang('th');
});

const MESSAGE = 'ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: USD ลิ้นชัก ขาด 7, EUR ตู้เซฟ ขาด 5.5';

function refusal(details?: unknown): ApiError {
  const err = new ApiError({ status: 409, code: 'stockInsufficient', message: MESSAGE });
  if (details !== undefined) Object.assign(err, { details });
  return err;
}

const TWO = {
  lines: [
    { currency: 'USD', place: 'DRAWER', shortBy: '7' },
    { currency: 'EUR', place: 'SAFE', shortBy: '5.5' },
  ],
};

it('S1: th names each short place, in order', () => {
  expect(wordedError(refusal(TWO))).toBe('เงินสดไม่พอ: USD ลิ้นชัก ขาด 7, EUR ตู้เซฟ ขาด 5.5 — โอนเงินเข้าก่อน แล้วลองใหม่');
});

it('S2: en words the same lines in English', () => {
  setLang('en');
  expect(wordedError(refusal(TWO))).toBe('Not enough cash: USD drawer short 7, EUR safe short 5.5 — transfer cash in first, then try again');
});

it('S3: a POSITION line reads as stock', () => {
  const pos = { lines: [{ currency: 'JPY', place: 'POSITION', shortBy: '1000' }] };
  expect(wordedError(refusal(pos))).toBe('เงินสดไม่พอ: JPY สต็อก ขาด 1000 — โอนเงินเข้าก่อน แล้วลองใหม่');
  setLang('en');
  expect(wordedError(refusal(pos))).toBe('Not enough cash: JPY stock short 1000 — transfer cash in first, then try again');
});

it('S4: no details, empty lines, or a malformed line ⇒ the shared text, never the server message', () => {
  const shared = (th as Record<string, string>)['error.stockInsufficient'];
  expect(wordedError(refusal())).toBe(shared);
  expect(wordedError(refusal({ lines: [] }))).toBe(shared);
  expect(wordedError(refusal({ lines: [{ currency: 'USD', place: 'NOWHERE', shortBy: '7' }] }))).toBe(shared);
  expect(wordedError(refusal({ lines: 'x' }))).toBe(shared);
  for (const d of [undefined, { lines: [] }]) expect(wordedError(refusal(d))).not.toBe(MESSAGE);
});

it('S5: another code with details is worded as before (details are read only for stockInsufficient)', () => {
  const err = new ApiError({ status: 409, code: 'ticketStillServing', message: 'm' });
  Object.assign(err, { details: TWO });
  expect(wordedError(err)).toBe((th as Record<string, string>)['error.ticketStillServing']);
});
