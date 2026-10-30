// formulaT reads `admin.parts.<key>` from parts/i18n.ts before its own table (shared/i18n screenT), so a key in both
// would show the parts text in a formula part. Guard for formula-i18n.ts's `formulaT` (pricing-formula-web.md §2).
import { describe, expect, it } from 'vitest';
import * as parts from './i18n';
import * as formula from './formula-i18n';

describe('pricing-formula-web: formula texts share the admin.parts namespace', () => {
  it('no formula key is also a parts key', () => {
    const partsKeys = new Set([...Object.keys(parts.th), ...Object.keys(parts.en)]);
    expect(Object.keys(formula.th).filter((k) => partsKeys.has(k))).toEqual([]);
  });

  it('formulaT renders a formula text', () => {
    expect(formula.formulaT('useDefault')).toBe(formula.th.useDefault);
  });
});
