// The vendored kiosk keeps all 16 languages of the shop's kiosk, and every text in each (api/static/kiosk/).
import { describe, expect, it } from 'vitest';
import { loadTexts } from './load-kiosk';

const SIXTEEN = ['en', 'th', 'zh', 'zh-HK', 'ja', 'ko', 'ru', 'hi', 'ar', 'lo', 'my', 'vi', 'ms', 'de', 'fr', 'id'];

describe('kiosk texts', () => {
  const { langs, text } = loadTexts();

  it('KIOSK_LANGS lists exactly the 16 languages', () => {
    expect(langs.map((l) => l.code).sort()).toEqual([...SIXTEEN].sort());
  });

  it('KIOSK_TEXT has the 16 languages', () => {
    expect(Object.keys(text).sort()).toEqual([...SIXTEEN].sort());
  });

  it.each(SIXTEEN)('every KIOSK_TEXT.en key has a non-empty %s text', (code) => {
    const missing = Object.keys(text.en).filter((key) => typeof text[code]?.[key] !== 'string' || text[code][key] === '');
    expect(missing).toEqual([]);
  });

  it('the branch control has its texts', () => {
    for (const key of ['branchLabel', 'branchSearch', 'chooseBranch']) expect(text.en[key]).toBeTruthy();
  });
});
