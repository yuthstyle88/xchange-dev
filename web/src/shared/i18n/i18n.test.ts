// V1-i18n (#427): the shared i18n module (docs/design/xchange-v3-web.md W10, §5.2).
// Thai is the default; English is the second language; the choice is kept per browser.
// Shared tables hold only field.* / form.* / shell.* / error.*; each screen owns
// web/src/screens/<path>/i18n.ts, collected by a recursive import.meta.glob and namespaced by its
// folder path with '/' as '.': screens/login → login.<key>, screens/admin/payments → admin.payments.<key>.
// Written before the implementation by the lane controller. Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createComponent, createMemo, createRoot } from 'solid-js';
import { render } from 'solid-js/web';
import { collectScreens, errorText, fill, lang, locale, screenT, setLang, t, translator } from './index';
import { LangSwitch } from './LangSwitch';
import { en } from './en';
import { th } from './th';

const STORAGE_KEY = 'xc.lang';

/** A copy of the module as a page that has just loaded it, reading storage afresh. */
async function freshModule() {
  vi.resetModules();
  return import('./index');
}

/** Every `{name}` in a table string. */
function placeholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort();
}

/**
 * The server's Thai refusal sentences by V3Error code: `PolicyError` (`xc-auth`
 * `domain/policy/value_objects/policy_error.rs`) and V0-B's `invalidRequest` / `serverError`
 * (`api/src/presentation/http/v3/view.rs`, #393). In Thai the screen shows exactly what it showed
 * before (W6), so the `th` table carries each one byte for byte.
 */
const THAI_SENTENCES: Record<string, string> = {
  signIn: 'กรุณาเข้าสู่ระบบ',
  ownerOnly: 'เฉพาะเจ้าของระบบเท่านั้น',
  invalidRequest: 'คำขอไม่ถูกต้อง',
  roleNotAllowed: 'บทบาทนี้ไม่มีสิทธิ์ทำรายการ',
  roleNotAllowedHere: 'บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้',
  ownerEnables: 'เจ้าของเป็นผู้เปิดใช้ขั้นตอน',
  frontOnly: 'ใช้ได้เฉพาะ Front',
  documentsNotReadable: 'บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้า',
  filesNotReadable: 'บทบาทนี้ไม่มีสิทธิ์อ่านเอกสารลูกค้าหรือหลักฐานเงิน',
  uploadNotAllowed: 'บทบาทนี้อัปโหลดหลักฐานไม่ได้',
  deviceStepNotAllowed: 'บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้',
  signNotAllowed: 'ไม่มีสิทธิ์ลงชื่อ',
  backendUsesCommit: 'Backend ใช้ปุ่มตรวจและบันทึกงาน',
  employeeAccount: 'ต้องใช้บัญชีพนักงาน',
  clockInFirst: 'ลงเวลาเข้าก่อนทำรายการ',
  paymentOwnerOnly: 'เฉพาะเจ้าของระบบเท่านั้นที่ตั้งค่าวิธีชำระเงินได้',
  drawerRoleNotAllowed: 'บัญชีนี้ไม่มีสิทธิ์เปิดลิ้นชัก',
  serverError: 'เซิร์ฟเวอร์ขัดข้อง',
};

/**
 * The five refusals the server sends in English. `th` must be Thai (lane controller 2026-10-08), so
 * `th` translates them (wording [proposed], read by the reviewer) and `en` keeps the server's
 * English. `employeeAuthRequired` is a code-like token on the server, so its `en` is only required
 * to be a sentence.
 */
const ENGLISH_ON_SERVER: Record<string, string> = {
  employeeAuthRequired: 'EMPLOYEE_AUTH_REQUIRED',
  notFound: 'Not found',
  kioskOwnerOrRates: 'Owner or Rates access required.',
  kioskStaff: 'Staff access required.',
  kioskUnknown: 'Unknown kiosk action.',
};

const THAI_SCRIPT = /[\u0E00-\u0E7F]/;
const SHARED_PREFIXES = ['field', 'form', 'shell', 'error', 'queue', 'rate'];

/** A screen module as `import.meta.glob(..., { eager: true })` hands it over. */
function screen(th: Record<string, string>, en: Record<string, string>) {
  return { th, en };
}

const FAKE_GLOB = {
  '/src/screens/login/i18n.ts': screen(
    { title: 'เข้าสู่ระบบ', 'submit.label': 'เข้าสู่ระบบ' },
    { title: 'Sign in', 'submit.label': 'Sign in' },
  ),
  '/src/screens/kiosk-currencies/i18n.ts': screen(
    { count: 'สกุลเงิน {n} รายการ', title: 'สกุลเงินที่ตู้' },
    { count: '{n} currencies', title: 'Kiosk currencies' },
  ),
};

let dispose: (() => void) | undefined;

beforeEach(() => {
  window.localStorage.clear();
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.restoreAllMocks();
  setLang('th');
  window.localStorage.clear();
  document.documentElement.removeAttribute('lang');
});

describe('t and lang', () => {
  it('a page that has stored nothing is in Thai, and t returns the th text', async () => {
    const i18n = await freshModule();
    expect(i18n.lang()).toBe('th');
    expect(i18n.t('field.password.show')).toBe(th['field.password.show']);
    expect(i18n.t('field.password.show')).toBe('แสดงรหัสผ่าน');
    expect(i18n.t('field.password.hide')).toBe('ซ่อนรหัสผ่าน');
    expect(document.documentElement.lang).toBe('th');
  });

  it('does not follow the browser language: navigator says en-US, the page is still Thai', async () => {
    vi.spyOn(window.navigator, 'language', 'get').mockReturnValue('en-US');
    vi.spyOn(window.navigator, 'languages', 'get').mockReturnValue(['en-US', 'en']);
    const i18n = await freshModule();
    expect(i18n.lang()).toBe('th');
    expect(document.documentElement.lang).toBe('th');
  });

  it("setLang('en') switches t, lang and <html lang>; setLang('th') switches back", () => {
    setLang('en');
    expect(lang()).toBe('en');
    expect(t('field.password.show')).toBe(en['field.password.show']);
    expect(t('field.password.show')).not.toBe(th['field.password.show']);
    expect(document.documentElement.lang).toBe('en');

    setLang('th');
    expect(lang()).toBe('th');
    expect(t('field.password.show')).toBe(th['field.password.show']);
    expect(document.documentElement.lang).toBe('th');
  });

  it('t is reactive: a computation that read it re-runs on a switch', () => {
    createRoot((done) => {
      const shown = createMemo(() => t('field.password.hide'));
      expect(shown()).toBe(th['field.password.hide']);
      setLang('en');
      expect(shown()).toBe(en['field.password.hide']);
      setLang('th');
      expect(shown()).toBe(th['field.password.hide']);
      done();
    });
  });

  it('locale follows lang: th-TH, then en-GB', () => {
    expect(locale()).toBe('th-TH');
    setLang('en');
    expect(locale()).toBe('en-GB');
    setLang('th');
    expect(locale()).toBe('th-TH');
  });
});

describe('placeholders', () => {
  it('fill puts each value in by its name, not by its position', () => {
    expect(fill('{b}-{a}-{b}', { a: '1', b: '2' })).toBe('2-1-2');
    expect(fill('{count} items', { count: 3 })).toBe('3 items');
  });

  it('fill leaves a placeholder without a value as it is, and ignores values it has no place for', () => {
    expect(fill('to {language}', {})).toBe('to {language}');
    expect(fill('to {language}')).toBe('to {language}');
    expect(fill('plain', { extra: 'x' })).toBe('plain');
  });

  it('fill does not expand a placeholder that arrives inside a value', () => {
    expect(fill('{a} and {b}', { a: '{b}', b: 'B' })).toBe('{b} and B');
  });

  it('t fills a table string by name in both languages', () => {
    const thText = t('shell.langSwitch', { language: 'Zz9' });
    expect(thText).toBe(fill(th['shell.langSwitch'], { language: 'Zz9' }));
    expect(thText).toContain('Zz9');
    expect(thText).not.toContain('{language}');

    setLang('en');
    const enText = t('shell.langSwitch', { language: 'Zz9' });
    expect(enText).toBe(fill(en['shell.langSwitch'], { language: 'Zz9' }));
    expect(enText).toContain('Zz9');
    expect(enText).not.toContain('{language}');
  });
});

describe('the choice is kept per browser', () => {
  it('setLang writes xc.lang, and a page loaded afterwards starts in that language', async () => {
    setLang('en');
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe('en');

    const i18n = await freshModule();
    expect(i18n.lang()).toBe('en');
    expect(i18n.locale()).toBe('en-GB');
    expect(document.documentElement.lang).toBe('en');
  });

  it('a stored value that is not th or en is read as th', async () => {
    for (const bad of ['fr', 'EN', '', ' en', 'null']) {
      window.localStorage.setItem(STORAGE_KEY, bad);
      const i18n = await freshModule();
      expect(i18n.lang(), JSON.stringify(bad)).toBe('th');
      expect(document.documentElement.lang).toBe('th');
    }
  });

  it('storage whose reads and writes throw: the page loads in Thai, and switching still works without throwing', async () => {
    window.localStorage.setItem(STORAGE_KEY, 'en');
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('blocked', 'SecurityError');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('full', 'QuotaExceededError');
    });

    const i18n = await freshModule();
    expect(i18n.lang()).toBe('th');
    expect(() => i18n.setLang('en')).not.toThrow();
    expect(i18n.lang()).toBe('en');
    expect(i18n.t('field.password.show')).toBe(en['field.password.show']);
    expect(document.documentElement.lang).toBe('en');
    expect(() => i18n.setLang('th')).not.toThrow();
    expect(i18n.lang()).toBe('th');
  });

  it('storage that cannot even be reached (the getter throws): Thai, and no throw', async () => {
    const own = Object.getOwnPropertyDescriptor(window, 'localStorage');
    Object.defineProperty(window, 'localStorage', {
      configurable: true,
      get() {
        throw new DOMException('denied', 'SecurityError');
      },
    });
    try {
      const i18n = await freshModule();
      expect(i18n.lang()).toBe('th');
      expect(() => i18n.setLang('en')).not.toThrow();
      expect(i18n.lang()).toBe('en');
    } finally {
      if (own) Object.defineProperty(window, 'localStorage', own);
    }
  });
});

describe('the two tables', () => {
  it('en and th have the same keys at runtime, and no empty text', () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(th).sort());
    for (const [key, text] of Object.entries(th)) {
      expect(typeof text, key).toBe('string');
      expect(text.trim(), key).not.toBe('');
    }
    for (const [key, text] of Object.entries(en)) {
      expect(typeof text, key).toBe('string');
      expect(text.trim(), key).not.toBe('');
    }
  });

  it('every key is dotted lowercase-first camelCase segments', () => {
    for (const key of Object.keys(th)) {
      expect(key, key).toMatch(/^[a-z][A-Za-z0-9]*(\.[a-z][A-Za-z0-9]*)+$/);
    }
  });

  it('every placeholder name in a th string is in its en string, and the other way round', () => {
    for (const key of Object.keys(th) as (keyof typeof th)[]) {
      expect(placeholders(en[key]), key).toEqual(placeholders(th[key]));
    }
  });

  it('the shared tables hold only field.*, form.*, shell.* and error.* keys', () => {
    for (const key of Object.keys(th)) {
      expect(SHARED_PREFIXES, key).toContain(key.split('.')[0]);
    }
  });

  it('every th text is Thai', () => {
    for (const [key, text] of Object.entries(th)) {
      expect(text, key).toMatch(THAI_SCRIPT);
    }
  });

  it('th carries each Thai server refusal sentence byte for byte under error.<code>, and en has its own', () => {
    const table = th as Record<string, string>;
    const english = en as Record<string, string>;
    for (const [code, sentence] of Object.entries(THAI_SENTENCES)) {
      expect(table[`error.${code}`], code).toBe(sentence);
      expect(english[`error.${code}`], code).toBeTruthy();
      expect(english[`error.${code}`], code).not.toMatch(THAI_SCRIPT);
    }
    expect(english['error.signIn']).not.toBe(table['error.signIn']);
    expect(english['error.invalidRequest']).not.toBe(table['error.invalidRequest']);
  });

  it('the five refusals the server sends in English are Thai in th, and keep the English in en', () => {
    const table = th as Record<string, string>;
    const english = en as Record<string, string>;
    for (const [code, sentence] of Object.entries(ENGLISH_ON_SERVER)) {
      expect(table[`error.${code}`], code).toMatch(THAI_SCRIPT);
      expect(table[`error.${code}`], code).not.toBe(sentence);
      expect(english[`error.${code}`], code).toBeTruthy();
      if (code !== 'employeeAuthRequired') expect(english[`error.${code}`], code).toBe(sentence);
    }
    expect(english['error.employeeAuthRequired']).not.toBe('EMPLOYEE_AUTH_REQUIRED');
  });
});

describe('errorText: a server refusal shown in the chosen language', () => {
  it('a code the tables know is shown from the table of the current language', () => {
    expect(errorText('signIn', 'from the server')).toBe(th['error.signIn']);
    expect(errorText('invalidRequest', 'from the server')).toBe(th['error.invalidRequest']);
    setLang('en');
    expect(errorText('signIn', 'from the server')).toBe(en['error.signIn']);
    expect(errorText('invalidRequest', 'from the server')).toBe(en['error.invalidRequest']);
  });

  it("a code the tables do not know is shown as the server's own message, in both languages", () => {
    expect(errorText('usernameTaken', 'ชื่อผู้ใช้นี้มีอยู่แล้ว')).toBe('ชื่อผู้ใช้นี้มีอยู่แล้ว');
    setLang('en');
    expect(errorText('usernameTaken', 'ชื่อผู้ใช้นี้มีอยู่แล้ว')).toBe('ชื่อผู้ใช้นี้มีอยู่แล้ว');
    expect(errorText('', 'empty code')).toBe('empty code');
  });

  it("a code that names an object property is not a table key: the server's message is shown", () => {
    for (const code of ['__proto__', 'constructor', 'toString', 'hasOwnProperty']) {
      expect(errorText(code, 'server text'), code).toBe('server text');
    }
  });
});

describe('screen tables: each screen owns its i18n.ts, collected by import.meta.glob', () => {
  it('t resolves <screen>.<key> per language from a glob of two screen modules, and still resolves shared keys', () => {
    const tr = translator(collectScreens(FAKE_GLOB));
    expect(tr('login.title')).toBe('เข้าสู่ระบบ');
    expect(tr('login.submit.label')).toBe('เข้าสู่ระบบ');
    expect(tr('kiosk-currencies.count', { n: 3 })).toBe('สกุลเงิน 3 รายการ');
    expect(tr('field.password.show')).toBe(th['field.password.show']);

    setLang('en');
    expect(tr('login.title')).toBe('Sign in');
    expect(tr('login.submit.label')).toBe('Sign in');
    expect(tr('kiosk-currencies.count', { n: 3 })).toBe('3 currencies');
    expect(tr('kiosk-currencies.title')).toBe('Kiosk currencies');
    expect(tr('field.password.show')).toBe(en['field.password.show']);
  });

  it('a screen key does not leak into another screen, and an unknown key shows as itself', () => {
    const tr = translator(collectScreens(FAKE_GLOB));
    expect(tr('kiosk-currencies.submit.label')).toBe('kiosk-currencies.submit.label');
    expect(tr('login.nope')).toBe('login.nope');
    expect(tr('nope.title')).toBe('nope.title');
  });

  it('is reactive like t: a computation that read it re-runs on a switch', () => {
    const tr = translator(collectScreens(FAKE_GLOB));
    createRoot((done) => {
      const shown = createMemo(() => tr('login.title'));
      expect(shown()).toBe('เข้าสู่ระบบ');
      setLang('en');
      expect(shown()).toBe('Sign in');
      done();
    });
  });

  it('a screen module whose en misses a key fails the parity check, naming the screen and the key', () => {
    const glob = {
      ...FAKE_GLOB,
      '/src/screens/login/i18n.ts': screen({ title: 'เข้าสู่ระบบ', 'submit.label': 'เข้าสู่ระบบ' }, { title: 'Sign in' }),
    };
    expect(() => collectScreens(glob)).toThrow(/login/);
    expect(() => collectScreens(glob)).toThrow(/submit\.label/);
  });

  it('a screen module whose en has a key th lacks fails the parity check', () => {
    const glob = { '/src/screens/login/i18n.ts': screen({ title: 'เข้าสู่ระบบ' }, { title: 'Sign in', extra: 'Extra' }) };
    expect(() => collectScreens(glob)).toThrow(/extra/);
  });

  it('a screen module whose placeholders differ between th and en fails the parity check', () => {
    const glob = { '/src/screens/devices/i18n.ts': screen({ count: 'มี {n} เครื่อง' }, { count: '{count} devices' }) };
    expect(() => collectScreens(glob)).toThrow(/devices/);
  });

  it('a module without both tables, a path outside src/screens/<screen>/i18n.ts, or a screen named like a shared prefix is refused', () => {
    expect(() => collectScreens({ '/src/screens/login/i18n.ts': { th: { title: 'เข้าสู่ระบบ' } } })).toThrow(/login/);
    expect(() => collectScreens({ '/src/screens/login/text.ts': screen({ a: 'ก' }, { a: 'A' }) })).toThrow();
    for (const name of SHARED_PREFIXES) {
      expect(() => collectScreens({ [`/src/screens/${name}/i18n.ts`]: screen({ a: 'ก' }, { a: 'A' }) }), name).toThrow();
    }
  });

  it('a nested folder is namespaced by its path: screens/admin/payments → admin.payments.<key>, per language', () => {
    const glob = {
      ...FAKE_GLOB,
      '/src/screens/admin/i18n.ts': screen({ menu: 'เมนู' }, { menu: 'Menu' }),
      '/src/screens/admin/payments/i18n.ts': screen(
        { title: 'วิธีชำระเงิน', saved: 'บันทึก {name} แล้ว' },
        { title: 'Payment methods', saved: '{name} saved' },
      ),
    };
    const tr = translator(collectScreens(glob));
    expect(tr('admin.payments.title')).toBe('วิธีชำระเงิน');
    expect(tr('admin.payments.saved', { name: 'QR' })).toBe('บันทึก QR แล้ว');
    expect(tr('admin.menu')).toBe('เมนู');
    expect(tr('login.title')).toBe('เข้าสู่ระบบ');
    setLang('en');
    expect(tr('admin.payments.title')).toBe('Payment methods');
    expect(tr('admin.payments.saved', { name: 'QR' })).toBe('QR saved');
    expect(tr('admin.menu')).toBe('Menu');
    expect(tr('payments.title')).toBe('payments.title');
  });

  it('screenT takes the dotted namespace of a nested folder', () => {
    const lt = screenT('zz-fixture.admin-payments', { th: { title: 'วิธีชำระเงิน' }, en: { title: 'Payment methods' } } as const);
    expect(lt('title')).toBe('วิธีชำระเงิน');
    setLang('en');
    expect(lt('title')).toBe('Payment methods');
    // @ts-expect-error — 'nope' is not a key of this table
    expect(lt('nope')).toBe('zz-fixture.admin-payments.nope');
  });

  it('a folder path segment that is not [a-z][a-z0-9-]* is refused', () => {
    for (const path of [
      '/src/screens/admin/Admin_X/i18n.ts',
      '/src/screens/Admin_X/i18n.ts',
      '/src/screens/admin/pay_ments/i18n.ts',
      '/src/screens/admin/9pay/i18n.ts',
      '/src/screens/admin/-pay/i18n.ts',
      '/src/screens/i18n.ts',
    ]) {
      expect(() => collectScreens({ [path]: screen({ a: 'ก' }, { a: 'A' }) }), path).toThrow();
    }
  });

  it('a key that two tables would both produce is refused, naming it', () => {
    const glob = {
      '/src/screens/admin/i18n.ts': screen({ 'payments.title': 'ก' }, { 'payments.title': 'A' }),
      '/src/screens/admin/payments/i18n.ts': screen({ title: 'ข' }, { title: 'B' }),
    };
    expect(() => collectScreens(glob)).toThrow(/admin\.payments\.title/);
  });

  it('an empty glob is fine: shared keys only', () => {
    const tr = translator(collectScreens({}));
    expect(tr('field.password.hide')).toBe(th['field.password.hide']);
  });

  it('every screen module in the repository passes the parity check', () => {
    const real = import.meta.glob('/src/screens/**/i18n.ts', { eager: true });
    expect(() => collectScreens(real)).not.toThrow();
  });
});

describe('screenT: the typed t a screen makes over its own table', () => {
  const loginI18n = {
    th: { title: 'เข้าสู่ระบบ', greet: 'สวัสดี {name}' },
    en: { title: 'Sign in', greet: 'Hello {name}' },
  } as const;

  it('resolves the screen key in the current language and fills placeholders by name', () => {
    const lt = screenT('zz-fixture-login', loginI18n);
    expect(lt('title')).toBe('เข้าสู่ระบบ');
    expect(lt('greet', { name: 'Ann' })).toBe('สวัสดี Ann');
    setLang('en');
    expect(lt('title')).toBe('Sign in');
    expect(lt('greet', { name: 'Ann' })).toBe('Hello Ann');
  });

  it('follows setLang reactively', () => {
    const lt = screenT('zz-fixture-login', loginI18n);
    createRoot((done) => {
      const shown = createMemo(() => lt('title'));
      expect(shown()).toBe('เข้าสู่ระบบ');
      setLang('en');
      expect(shown()).toBe('Sign in');
      setLang('th');
      expect(shown()).toBe('เข้าสู่ระบบ');
      done();
    });
  });

  it('a key the table does not have is a compile error, and at runtime shows as <screen>.<key>', () => {
    const lt = screenT('zz-fixture-login', loginI18n);
    // @ts-expect-error — 'nope' is not a key of loginI18n.th
    const shown = lt('nope');
    expect(shown).toBe('zz-fixture-login.nope');
  });
});

describe('LangSwitch', () => {
  function mount() {
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => createComponent(LangSwitch, {}), host);
    const buttons = host.querySelectorAll('button');
    expect(buttons).toHaveLength(1);
    return { host, button: buttons[0]! };
  }

  it('is one plain button that toggles th and en, with <html lang> and storage following', () => {
    const { button } = mount();
    expect(button.getAttribute('type')).toBe('button');
    expect(lang()).toBe('th');

    button.click();
    expect(lang()).toBe('en');
    expect(document.documentElement.lang).toBe('en');
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe('en');

    button.click();
    expect(lang()).toBe('th');
    expect(document.documentElement.lang).toBe('th');
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe('th');
  });

  it('shows the language it switches to, and its aria-label comes from the table of the current language', () => {
    const { button } = mount();
    expect(button.textContent?.trim()).toBe('EN');
    const thLabel = button.getAttribute('aria-label');
    expect(thLabel).toBe(t('shell.langSwitch', { language: 'English' }));

    button.click();
    expect(button.textContent?.trim()).toBe('TH');
    const enLabel = button.getAttribute('aria-label');
    expect(enLabel).toBe(t('shell.langSwitch', { language: 'ไทย' }));
    expect(enLabel).not.toBe(thLabel);
  });

  it('follows a switch made elsewhere', () => {
    const { button } = mount();
    setLang('en');
    expect(button.textContent?.trim()).toBe('TH');
    setLang('th');
    expect(button.textContent?.trim()).toBe('EN');
  });
});
