// Written before the implementation by the ticket author claude/5edf7bb5. Do not edit, delete or weaken.
import { afterEach, expect, it } from 'vitest';
import { ApiError } from '../api/client';
import { setAudience, setLang } from './index';
import { en } from './en';
import { th } from './th';
import { zh } from './zh';
import { wordedError } from './worded-error';

afterEach(() => {
  setAudience('staff');
  setLang('th');
});

it('a 409 stockInsufficient reads as the shared text in th, en and zh, never the server message', () => {
  const err = new ApiError({
    status: 409,
    code: 'stockInsufficient',
    message: 'stockInsufficient: the posting takes a drawer, a safe or a position below zero',
  });
  const table = (t: object) => (t as Record<string, string>)['error.stockInsufficient'];
  expect(table(th)).toBeTruthy();
  expect(wordedError(err)).toBe(table(th));
  setLang('en');
  expect(wordedError(err)).toBe(table(en));
  setAudience('customer'); // zh is a customer-page language only (languages.ts SPOKEN); staff pages are th/en
  setLang('zh');
  expect(wordedError(err)).toBe(table(zh));
});
