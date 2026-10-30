// Pricing formula FE T2 up-front tests (docs/design/blueprint/pricing-formula-web.md §3 S1–S10), written before the
// implementation by the sub-controller claude/studio-claude-3. Do not edit, delete or weaken; add tests in a file of
// your own. The line grammar is `SpreadFailure::describe` (api/crates/xc-rate/src/application/formula/spread_check.rs:92-130).
// S1–S4 use, verbatim, the four sample lines of describe()'s doc comment (spread_check.rs:92-94), so the two sides pin
// the same strings (#5570 verdict follow-up, issuecomment-6513014829). Change them only together with that comment.
import { beforeEach, describe, expect, it } from 'vitest';
import { ApiError } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { failureText, parseSpreadFailure, spreadRefusal } from './spread-failure';

function refusal(over: Partial<{ status: number; code: string; field: string; message: string }> = {}): ApiError {
  return new ApiError({
    status: 400,
    code: 'validationFailed',
    field: 'buyFormula',
    message: 'MAIN USD buy 33.8837374 > sell 33.7293303',
    ...over,
  });
}

describe('pricing-formula-web T2: parseSpreadFailure', () => {
  it('S1: a single-price buy > sell line', () => {
    expect(parseSpreadFailure('MAIN USD buy 33.8837374 > sell 33.7293303')).toEqual({
      branch: 'MAIN',
      currency: 'USD',
      tier: null,
      follow: false,
      kind: 'buyAboveSell',
      buy: '33.8837374',
      sell: '33.7293303',
      side: null,
      guardMin: null,
      guardMax: null,
      pct: null,
      code: null,
      raw: 'MAIN USD buy 33.8837374 > sell 33.7293303',
    });
  });

  it('S2: a tier and FOLLOW', () => {
    const line = parseSpreadFailure('MAIN USD tier 3 FOLLOW buy 1.1 > sell 1.0');
    expect(line).not.toBeNull();
    expect(line!.tier).toBe(3);
    expect(line!.follow).toBe(true);
    expect(line!.kind).toBe('buyAboveSell');
    expect(line!.buy).toBe('1.1');
    expect(line!.sell).toBe('1.0');
  });

  it('S3: a formula error code', () => {
    const line = parseSpreadFailure('B2 JPY tier 1 divideByZero');
    expect(line).toMatchObject({
      branch: 'B2',
      currency: 'JPY',
      tier: 1,
      follow: false,
      kind: 'formula',
      code: 'divideByZero',
      buy: null,
      sell: null,
    });
  });

  it('S3: every FormulaError code is a formula line', () => {
    for (const code of [
      'syntax',
      'unknownInput',
      'missingInput',
      'divideByZero',
      'overflow',
      'notPositive',
      'tooLong',
      'tooDeep',
    ]) {
      expect(parseSpreadFailure(`MAIN EUR ${code}`)?.code, code).toBe(code);
    }
  });

  it('S4: outside the guard', () => {
    expect(parseSpreadFailure('MAIN USD buy 34.1 outside guard [-1, 1] (+1.2 %)')).toMatchObject({
      kind: 'outsideGuard',
      side: 'buy',
      buy: '34.1',
      sell: null,
      guardMin: '-1',
      guardMax: '1',
      pct: '+1.2',
    });
    expect(parseSpreadFailure('S1 GBP tier 2 sell 40.5 outside guard [-0.5, 0.5] (-0.7 %)')).toMatchObject({
      tier: 2,
      kind: 'outsideGuard',
      side: 'sell',
      sell: '40.5',
      buy: null,
      guardMin: '-0.5',
      guardMax: '0.5',
      pct: '-0.7',
    });
  });

  it('S5: a missing price is printed as "-" and read as null', () => {
    expect(parseSpreadFailure('MAIN USD buy - > sell 33.7')).toMatchObject({ buy: null, sell: '33.7' });
  });

  it('S6: text that is not a failure line', () => {
    expect(parseSpreadFailure('unknown input foo')).toBeNull();
    expect(parseSpreadFailure('MAIN USD hello')).toBeNull();
    expect(parseSpreadFailure('')).toBeNull();
  });
});

describe('pricing-formula-web T2: spreadRefusal', () => {
  it('S7: every line of a check refusal, in order', () => {
    const lines = spreadRefusal(
      refusal({ message: 'MAIN USD buy 33.8837374 > sell 33.7293303\nB2 JPY tier 4 FOLLOW buy 0.3 > sell 0.2' }),
    );
    expect(lines?.map((l) => [l.branch, l.currency, l.tier, l.follow])).toEqual([
      ['MAIN', 'USD', null, false],
      ['B2', 'JPY', 4, true],
    ]);
  });

  it('S7: ratePolicySave refuses on buyAdjustPct with the same lines', () => {
    expect(spreadRefusal(refusal({ field: 'buyAdjustPct' }))).toHaveLength(1);
  });

  it('S8: one line that does not parse ⇒ not a check refusal', () => {
    expect(spreadRefusal(refusal({ message: 'MAIN USD buy 1 > sell 0.9\nunknown input `foo`' }))).toBeNull();
    expect(spreadRefusal(refusal({ field: 'sellFormula', message: 'unknown input `foo`' }))).toBeNull();
  });

  it('S9: other errors are not a check refusal', () => {
    expect(spreadRefusal(refusal({ status: 403, code: 'roleNotAllowedHere' }))).toBeNull();
    expect(spreadRefusal(refusal({ status: 500, code: 'serverError' }))).toBeNull();
    expect(spreadRefusal(refusal({ code: 'conflict' }))).toBeNull();
    expect(spreadRefusal(new Error('MAIN USD buy 1 > sell 0.9'))).toBeNull();
    expect(spreadRefusal(null)).toBeNull();
  });
});

describe('pricing-formula-web T2: failureText', () => {
  beforeEach(() => setLang('th'));

  it('S10: buy above sell names both prices', () => {
    const text = failureText(parseSpreadFailure('MAIN USD buy 33.8837374 > sell 33.7293303')!);
    expect(text).toBe('ราคารับซื้อ 33.8837374 สูงกว่าราคาขาย 33.7293303');
  });

  it('S10: a formula error uses its code text', () => {
    expect(failureText(parseSpreadFailure('B2 JPY tier 1 divideByZero')!)).toBe('สูตรหารด้วยศูนย์');
    expect(failureText(parseSpreadFailure('B2 JPY notPositive')!)).toBe('ผลลัพธ์ต้องมากกว่า 0');
  });

  it('S10: outside the guard names the side, price, band and distance', () => {
    expect(failureText(parseSpreadFailure('MAIN USD buy 34.1 outside guard [-1, 1] (+1.2 %)')!)).toBe(
      'ราคารับซื้อ 34.1 อยู่นอกกรอบ -1 ถึง 1 (+1.2 %)',
    );
    expect(failureText(parseSpreadFailure('MAIN USD sell 40.5 outside guard [-0.5, 0.5] (-0.7 %)')!)).toBe(
      'ราคาขาย 40.5 อยู่นอกกรอบ -0.5 ถึง 0.5 (-0.7 %)',
    );
  });

  it('S10: a missing price shows as a dash', () => {
    expect(failureText(parseSpreadFailure('MAIN USD buy - > sell 33.7')!)).toBe('ราคารับซื้อ — สูงกว่าราคาขาย 33.7');
  });
});
