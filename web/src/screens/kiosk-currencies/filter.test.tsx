// V1-kiosk-currencies (#399): the search of the currency screen (ticket §7.4 `filter.ts`, §9,
// §10.1 C5). Written before the implementation by the lane controller. Do not edit, delete or
// weaken; add tests in a file of your own.
//
// v2 (`kiosk-currencies.html` line 3): join [code, name, nameEn, ...countries.flatMap(c =>
// [c.code, c.en, c.th])] with spaces, lower-case it, and keep the currency when it includes
// query.trim().toLowerCase().
//
// Amended by the ticket author of #603 (RE-6, claude/e5ab6c; `docs/design/blueprint/rates.md`
// §4.9 R1, §9 RE-6): `currency()` fills the view's new `unitSize` with `'1'`. The search does not
// read it.
import { describe, expect, it } from 'vitest';
import type { KioskCurrency } from '../../shared/api/generated/kiosk_currencies';
import { matches } from './filter';

function currency(
  code: string,
  name: string,
  nameEn: string,
  countries: Array<[string, string, string]>,
): KioskCurrency {
  return {
    code,
    name,
    nameEn,
    countries: countries.map(([c, en, th]) => ({ code: c, en, th })),
    buy: null,
    sell: null,
    unitSize: '1',
    updated: '',
    partnerName: '',
    partnerCost: null,
    enabled: false,
    kbank: [],
  };
}

const JPY = currency('JPY', 'เยนญี่ปุ่น', 'Japanese Yen', [['JP', 'Japan', 'ญี่ปุ่น']]);
const USD = currency('USD', 'ดอลลาร์สหรัฐ', 'US Dollar', [
  ['US', 'United States', 'สหรัฐอเมริกา'],
  ['EC', 'Ecuador', 'เอกวาดอร์'],
]);
const EUR = currency('EUR', 'ยูโร', 'Euro', [
  ['AT', 'Austria', 'ออสเตรีย'],
  ['DE', 'Germany', 'เยอรมนี'],
]);
const ALL = [JPY, USD, EUR];

function codes(query: string): string[] {
  return ALL.filter((c) => matches(c, query)).map((c) => c.code);
}

describe('matches (#399 C5)', () => {
  it('finds by a country in English, in Thai, and by a country code (§9)', () => {
    expect(codes('japan')).toEqual(['JPY']);
    expect(codes('ญี่ปุ่น')).toEqual(['JPY']);
    expect(codes('JP')).toEqual(['JPY']);
    expect(codes('ecuador')).toEqual(['USD']);
    expect(codes('เยอรมนี')).toEqual(['EUR']);
    expect(codes('de')).toEqual(['EUR']);
  });

  it('finds by code, Thai name and English name, whatever the case', () => {
    expect(codes('usd')).toEqual(['USD']);
    expect(codes('UsD')).toEqual(['USD']);
    expect(codes('ยูโร')).toEqual(['EUR']);
    expect(codes('euro')).toEqual(['EUR']);
    expect(codes('JAPANESE yen')).toEqual(['JPY']);
  });

  it('is a substring match', () => {
    expect(codes('dollar')).toEqual(['USD']);
    expect(codes('stria')).toEqual(['EUR']);
  });

  it('trims the query, and an empty query keeps everything', () => {
    expect(codes('  usd  ')).toEqual(['USD']);
    expect(codes('')).toEqual(['JPY', 'USD', 'EUR']);
    expect(codes('   ')).toEqual(['JPY', 'USD', 'EUR']);
  });

  it('matches across the fields as v2 joined them, with one space between', () => {
    expect(codes('usd ดอลลาร์สหรัฐ')).toEqual(['USD']);
    expect(codes('japanese yen jp')).toEqual(['JPY']);
    expect(codes('usdดอลลาร์')).toEqual([]);
  });

  it('a query nothing holds keeps nothing', () => {
    expect(codes('zzz')).toEqual([]);
    expect(codes('thb')).toEqual([]);
  });
});
