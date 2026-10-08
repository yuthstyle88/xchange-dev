import { afterEach, describe, expect, it } from 'vitest';
import indexSource from './index.ts?raw';
import { buildLookup, lang, screenLookup, screenT, setLang } from './index';

afterEach(() => {
  setLang('th');
});

describe('i18n load path: real glob and screenLookup', () => {
  it('index.ts imports the real recursive glob at module load with the exact pattern', () => {
    expect(indexSource).toContain("import.meta.glob('/src/screens/**/i18n.ts', { eager: true })");
    expect(indexSource).toMatch(/buildLookup\(\s*import\.meta\.glob\('/);
  });

  it('screenLookup is built at module load and exported', () => {
    expect(screenLookup).toBeDefined();
    expect(screenLookup.th).toBeDefined();
    expect(screenLookup.en).toBeDefined();
    // It is what buildLookup builds from the real glob, whether there are zero screen tables or many (#441)
    const expected = buildLookup(import.meta.glob('/src/screens/**/i18n.ts', { eager: true }));
    expect(screenLookup.th).toEqual(expected.th);
    expect(screenLookup.en).toEqual(expected.en);
  });

  it('buildLookup enforces collectScreens parity and segment checks', () => {
    // Missing en key throws naming screen and key
    expect(() =>
      buildLookup({
        '/src/screens/login/i18n.ts': {
          th: { title: 'เข้าสู่ระบบ', submit: 'ตกลง' },
          en: { title: 'Sign in' },
        },
      }),
    ).toThrow(/login/);

    // Mismatched placeholders throw naming screen
    expect(() =>
      buildLookup({
        '/src/screens/devices/i18n.ts': {
          th: { count: 'มี {n} เครื่อง' },
          en: { count: '{count} devices' },
        },
      }),
    ).toThrow(/devices/);

    // Invalid folder segment throws
    expect(() =>
      buildLookup({
        '/src/screens/admin/Admin_X/i18n.ts': {
          th: { a: 'ก' },
          en: { a: 'A' },
        },
      }),
    ).toThrow();

    // Duplicate collision throws
    expect(() =>
      buildLookup({
        '/src/screens/admin/i18n.ts': {
          th: { 'payments.title': 'ก' },
          en: { 'payments.title': 'A' },
        },
        '/src/screens/admin/payments/i18n.ts': {
          th: { title: 'ข' },
          en: { title: 'B' },
        },
      }),
    ).toThrow(/admin\.payments\.title/);
  });

  it('screenT resolves through screenLookup rather than the passed table directly', () => {
    // Populate screenLookup temporarily
    screenLookup.th['mock-screen.title'] = 'ข้อความจาก lookup';
    screenLookup.en['mock-screen.title'] = 'Text from lookup';

    try {
      const lt = screenT('mock-screen', {
        th: { title: 'ข้อความจาก table' },
        en: { title: 'Text from table' },
      });

      // Resolves from screenLookup, NOT the passed table
      expect(lt('title')).toBe('ข้อความจาก lookup');
      expect(lt('title')).not.toBe('ข้อความจาก table');

      setLang('en');
      expect(lang()).toBe('en');
      expect(lt('title')).toBe('Text from lookup');
      expect(lt('title')).not.toBe('Text from table');

      setLang('th');
      expect(lt('title')).toBe('ข้อความจาก lookup');
    } finally {
      delete screenLookup.th['mock-screen.title'];
      delete screenLookup.en['mock-screen.title'];
    }
  });

  it('screenT falls back to table when key is not in screenLookup (for unit tests)', () => {
    const lt = screenT('in-memory-screen', {
      th: { item: 'รายการ' },
      en: { item: 'Item' },
    });
    expect(lt('item')).toBe('รายการ');
    setLang('en');
    expect(lt('item')).toBe('Item');
  });

  it('screenT returns <screen>.<key> when key is not in lookup or table', () => {
    const lt = screenT('mock', {
      th: { a: 'ก' },
      en: { a: 'A' },
    });
    // @ts-expect-error missing key
    expect(lt('unknown')).toBe('mock.unknown');
  });
});
