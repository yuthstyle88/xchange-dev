import { createSignal, type Accessor } from 'solid-js';
import { en } from './en';
import { th, type Key } from './th';
import { zh } from './zh';
import {
  CUSTOMER_ROOT,
  HTML_LANG,
  LOCALE,
  SPOKEN,
  type Audience,
  type Lang,
} from './languages';

export type { Audience, Lang } from './languages';
export type Locale = (typeof LOCALE)[Lang];
export type Params = Record<string, string | number>;

const STORAGE_KEY = 'xc.lang';
const SHARED_PREFIXES = ['field', 'form', 'shell', 'error', 'queue', 'rate', 'kioskCancel', 'input'];
const SEGMENT_REGEX = /^_?[a-z][a-z0-9-]*$/;

function readStorage(): Lang {
  try {
    if (typeof window !== 'undefined' && window.localStorage) {
      const val = window.localStorage.getItem(STORAGE_KEY);
      if (val === 'th' || val === 'en' || val === 'zh') return val;
    }
  } catch {
    // storage blocked or getter throws
  }
  return 'th';
}

function syncHtmlLang(l: Lang): void {
  try {
    if (typeof document !== 'undefined' && document.documentElement) {
      document.documentElement.lang = HTML_LANG[l];
    }
  } catch {
    // ignore
  }
}

const [storedChoice, setStoredChoice] = createSignal<Lang>(readStorage());
const [currentAudience, setCurrentAudience] = createSignal<Audience>('staff');

export const audience: Accessor<Audience> = currentAudience;

export const lang: Accessor<Lang> = () => {
  const choice = storedChoice();
  const allowed = SPOKEN[currentAudience()];
  return allowed.includes(choice) ? choice : 'th';
};

export const locale: Accessor<Locale> = () => LOCALE[lang()];

// Sync initial HTML lang
syncHtmlLang(lang());

export function setAudience(a: Audience): void {
  setCurrentAudience(a);
  syncHtmlLang(lang());
}

export function setLang(l: Lang): void {
  const allowed = SPOKEN[currentAudience()];
  if (!allowed.includes(l)) return;
  setStoredChoice(l);
  syncHtmlLang(l);
  try {
    if (typeof window !== 'undefined' && window.localStorage) {
      window.localStorage.setItem(STORAGE_KEY, l);
    }
  } catch {
    // storage quota exceeded or blocked
  }
}

export function fill(template: string, params?: Params): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (match, name) => {
    if (Object.prototype.hasOwnProperty.call(params, name) && params[name] !== undefined) {
      return String(params[name]);
    }
    return match;
  });
}

export function t(key: Key, params?: Params): string {
  const l = lang();
  let text: string;
  if (l === 'zh') {
    text = (zh as Record<string, string>)[key] ?? (en as Record<string, string>)[key] ?? key;
  } else if (l === 'en') {
    text = en[key] ?? key;
  } else {
    text = th[key] ?? key;
  }
  return fill(text, params);
}

export function errorText(code: string, message: string): string {
  const key = `error.${code}` as Key;
  const l = lang();
  if (l === 'zh') {
    if (Object.prototype.hasOwnProperty.call(zh, key) && (zh as Record<string, string>)[key]) {
      return (zh as Record<string, string>)[key];
    }
    if (Object.prototype.hasOwnProperty.call(en, key) && (en as Record<string, string>)[key]) {
      return (en as Record<string, string>)[key];
    }
    return message;
  }
  if (Object.prototype.hasOwnProperty.call(th, key)) {
    return t(key);
  }
  return message;
}

export interface ScreenTable {
  readonly th: Record<string, string>;
  readonly en: Record<string, string>;
  readonly zh?: Record<string, string>;
}

export interface ScreenTables {
  th: Record<string, string>;
  en: Record<string, string>;
  zh: Record<string, string>;
}

function extractPlaceholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort();
}

export function collectScreens(
  glob: Record<string, unknown>,
): ScreenTables {
  const result: ScreenTables = { th: {}, en: {}, zh: {} };

  for (const [path, mod] of Object.entries(glob)) {
    const match = path.match(/^\/src\/screens\/(.+)\/i18n\.ts$/);
    if (!match) {
      throw new Error(`Invalid screen i18n path: "${path}"`);
    }

    const segments = match[1].split('/');
    if (segments.length === 0 || segments.some((s) => s.length === 0)) {
      throw new Error(`Invalid screen path: "${path}"`);
    }

    for (const segment of segments) {
      if (!SEGMENT_REGEX.test(segment)) {
        throw new Error(`Invalid screen path segment "${segment}" in "${path}"`);
      }
    }

    if (SHARED_PREFIXES.includes(segments[0])) {
      throw new Error(`Screen prefix cannot be a shared prefix "${segments[0]}" in "${path}"`);
    }

    const screenName = segments.join('.');
    const isCustomer = segments[0] === CUSTOMER_ROOT;

    const m = mod as {
      th?: Record<string, string>;
      en?: Record<string, string>;
      zh?: Record<string, string>;
    } | null | undefined;

    if (
      !m ||
      typeof m !== 'object' ||
      !m.th ||
      typeof m.th !== 'object' ||
      !m.en ||
      typeof m.en !== 'object'
    ) {
      throw new Error(`Screen "${screenName}" must export both th and en tables in "${path}"`);
    }

    const thKeys = Object.keys(m.th).sort();
    const enKeys = Object.keys(m.en).sort();

    for (const k of thKeys) {
      if (!Object.prototype.hasOwnProperty.call(m.en, k)) {
        throw new Error(`Screen "${screenName}" key "${k}" is missing in en`);
      }
    }

    for (const k of enKeys) {
      if (!Object.prototype.hasOwnProperty.call(m.th, k)) {
        throw new Error(`Screen "${screenName}" key "${k}" is missing in th`);
      }
    }

    for (const k of thKeys) {
      const thP = extractPlaceholders(m.th[k]);
      const enP = extractPlaceholders(m.en[k]);
      if (thP.join(',') !== enP.join(',')) {
        throw new Error(`Screen "${screenName}" placeholders mismatch for key "${k}": [${thP}] vs [${enP}]`);
      }
    }

    if (isCustomer) {
      if (!m.zh || typeof m.zh !== 'object') {
        throw new Error(`Screen "${screenName}" is a customer screen and must export zh table in "${path}"`);
      }

      const zhKeys = Object.keys(m.zh).sort();

      for (const k of thKeys) {
        if (!Object.prototype.hasOwnProperty.call(m.zh, k)) {
          throw new Error(`Screen "${screenName}" key "${k}" is missing in zh`);
        }
      }

      for (const k of zhKeys) {
        if (!Object.prototype.hasOwnProperty.call(m.th, k)) {
          throw new Error(`Screen "${screenName}" key "${k}" in zh is not in th (extra key)`);
        }
      }

      for (const k of thKeys) {
        const val = m.zh[k];
        if (typeof val !== 'string' || val.trim() === '') {
          throw new Error(`Screen "${screenName}" key "${k}" in zh has empty value`);
        }
        const thP = extractPlaceholders(m.th[k]);
        const zhP = extractPlaceholders(val);
        if (thP.join(',') !== zhP.join(',')) {
          throw new Error(`Screen "${screenName}" placeholders mismatch for key "${k}" in zh: [${thP}] vs [${zhP}]`);
        }
      }

      for (const k of thKeys) {
        const namespacedKey = `${screenName}.${k}`;
        result.zh[namespacedKey] = m.zh[k];
      }
    } else {
      if (m.zh !== undefined) {
        throw new Error(`Screen "${screenName}" is a staff screen: TH/EN only (must not export zh) in "${path}"`);
      }
    }

    for (const k of thKeys) {
      const namespacedKey = `${screenName}.${k}`;
      if (Object.prototype.hasOwnProperty.call(result.th, namespacedKey)) {
        throw new Error(`Duplicate screen key: ${namespacedKey}`);
      }
      result.th[namespacedKey] = m.th[k];
      result.en[namespacedKey] = m.en[k];
    }
  }

  return result;
}

export function buildLookup(globResult: Record<string, unknown>): ScreenTables {
  return collectScreens(globResult);
}

export const screenLookup: ScreenTables = buildLookup(
  import.meta.glob('/src/screens/**/i18n.ts', { eager: true }),
);

export function translator(
  screens: ScreenTables,
): (key: string, params?: Params) => string {
  return (key: string, params?: Params): string => {
    const l = lang();
    if (l === 'zh') {
      if (screens.zh && Object.prototype.hasOwnProperty.call(screens.zh, key) && screens.zh[key]) {
        return fill(screens.zh[key], params);
      }
      if (screens.en && Object.prototype.hasOwnProperty.call(screens.en, key) && screens.en[key]) {
        return fill(screens.en[key], params);
      }
      const zhShared = (zh as Record<string, string>)[key];
      if (zhShared) {
        return fill(zhShared, params);
      }
      const enShared = (en as Record<string, string>)[key];
      if (enShared) {
        return fill(enShared, params);
      }
      return key;
    }
    const table = l === 'th' ? screens.th : screens.en;
    if (Object.prototype.hasOwnProperty.call(table, key)) {
      return fill(table[key], params);
    }
    const sharedTable = l === 'th' ? th : en;
    if (Object.prototype.hasOwnProperty.call(sharedTable, key)) {
      return fill((sharedTable as Record<string, string>)[key], params);
    }
    return key;
  };
}

export function screenT<
  T extends ScreenTable,
>(
  screen: string,
  table: T,
): (key: keyof T['th'] & string, params?: Params) => string {
  return (key: keyof T['th'] & string, params?: Params): string => {
    const l = lang();
    const namespacedKey = `${screen}.${String(key)}`;
    if (l === 'zh') {
      if (screenLookup.zh && Object.prototype.hasOwnProperty.call(screenLookup.zh, namespacedKey) && screenLookup.zh[namespacedKey]) {
        return fill(screenLookup.zh[namespacedKey], params);
      }
      if (table.zh && Object.prototype.hasOwnProperty.call(table.zh, key) && table.zh[key as keyof typeof table.zh]) {
        return fill((table.zh as Record<string, string>)[key as string], params);
      }
      if (screenLookup.en && Object.prototype.hasOwnProperty.call(screenLookup.en, namespacedKey) && screenLookup.en[namespacedKey]) {
        return fill(screenLookup.en[namespacedKey], params);
      }
      if (table.en && Object.prototype.hasOwnProperty.call(table.en, key) && table.en[key as keyof typeof table.en]) {
        return fill((table.en as Record<string, string>)[key as string], params);
      }
      return namespacedKey;
    }
    const collectedTable = l === 'th' ? screenLookup.th : screenLookup.en;
    if (Object.prototype.hasOwnProperty.call(collectedTable, namespacedKey)) {
      return fill(collectedTable[namespacedKey], params);
    }
    const currentTable = l === 'th' ? table.th : table.en;
    if (currentTable && Object.prototype.hasOwnProperty.call(currentTable, key)) {
      return fill((currentTable as Record<string, string>)[key as string], params);
    }
    return namespacedKey;
  };
}
