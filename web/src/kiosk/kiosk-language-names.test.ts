import { beforeAll, describe, expect, it } from 'vitest';
import { el, fakeApi, openKiosk } from './load-kiosk';

describe('kiosk language list', () => {
  beforeAll(() => {
    openKiosk('?branch=MAIN', fakeApi());
  });

  it('shows language names without rank or #, preserving initial order', () => {
    const options = [...el<HTMLSelectElement>('lang').options];
    const langs = (window as unknown as { KIOSK_LANGS: Array<{ code: string; name: string }> }).KIOSK_LANGS;
    const nameByCode = new Map(langs.map((l) => [l.code, l.name]));

    expect(options.length).toBe(langs.length);
    for (const option of options) {
      expect(option.text).toBe(nameByCode.get(option.value));
      expect(option.text).not.toContain('#');
    }
    expect(options.slice(0, 3).map((o) => o.value)).toEqual(['ms', 'zh', 'hi']);
  });
});
