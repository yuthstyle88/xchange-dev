import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { en, th } from './i18n';
import { flagLabel, statusLabel, type PricingFlagCode, type RowStatusCode } from './pricing-labels';

const ALL_STATUS_CODES: readonly RowStatusCode[] = [
  'Current',
  'Pending',
  'Unpriced',
  'Tiers',
  'Review',
  'Manual',
  'Reference',
];

const ALL_FLAG_CODES: readonly PricingFlagCode[] = [
  'NoAnchor',
  'NoTtQuote',
  'StaleSource',
  'MinSpread',
  'BelowCost',
  'LargeMove',
  'Precision',
];

describe('pricing-labels', () => {
  beforeEach(() => {
    setLang('th');
  });

  afterEach(() => {
    setLang('th');
  });

  describe('i18n key parity', () => {
    it('th and en have the exact same key set', () => {
      const thKeys = Object.keys(th).sort();
      const enKeys = Object.keys(en).sort();
      expect(thKeys).toEqual(enKeys);
    });

    it('all status.<Code> and flag.<Code> keys exist with non-empty values', () => {
      for (const code of ALL_STATUS_CODES) {
        const key = `status.${code}` as keyof typeof th;
        expect(th[key]).toBeTruthy();
        expect(en[key]).toBeTruthy();
      }
      for (const code of ALL_FLAG_CODES) {
        const key = `flag.${code}` as keyof typeof th;
        expect(th[key]).toBeTruthy();
        expect(en[key]).toBeTruthy();
      }
    });
  });

  describe('statusLabel', () => {
    it.each(ALL_STATUS_CODES)('returns non-empty label for status %s in TH and EN', (status) => {
      setLang('th');
      const thLabel = statusLabel(status);
      expect(thLabel).toBe(th[`status.${status}` as keyof typeof th]);
      expect(thLabel.length).toBeGreaterThan(0);

      setLang('en');
      const enLabel = statusLabel(status);
      expect(enLabel).toBe(en[`status.${status}` as keyof typeof en]);
      expect(enLabel.length).toBeGreaterThan(0);
    });

    it('returns empty string for unknown status code', () => {
      expect(statusLabel('Unknown' as RowStatusCode)).toBe('');
    });
  });

  describe('flagLabel', () => {
    it.each(ALL_FLAG_CODES)('returns non-empty label for flag %s in TH and EN', (flag) => {
      setLang('th');
      const thLabel = flagLabel(flag);
      expect(thLabel).toBe(th[`flag.${flag}` as keyof typeof th]);
      expect(thLabel.length).toBeGreaterThan(0);

      setLang('en');
      const enLabel = flagLabel(flag);
      expect(enLabel).toBe(en[`flag.${flag}` as keyof typeof en]);
      expect(enLabel.length).toBeGreaterThan(0);
    });

    it('returns empty string for unknown flag code', () => {
      expect(flagLabel('Unknown' as PricingFlagCode)).toBe('');
    });
  });
});
