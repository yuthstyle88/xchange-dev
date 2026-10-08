import { createSignal, type Accessor } from 'solid-js';
import { en } from './en';
import { th, type Key } from './th';

export type Lang = 'th' | 'en';
export type Locale = 'th-TH' | 'en-GB';
export type Params = Record<string, string | number>;

const STORAGE_KEY = 'xc.lang';
const SHARED_PREFIXES = ['field', 'form', 'shell', 'error'];
const SEGMENT_REGEX = /^[a-z][a-z0-9-]*$/;

function readStorage(): Lang {
  try {
    if (typeof window !== 'undefined' && window.localStorage) {
      const val = window.localStorage.getItem(STORAGE_KEY);
      if (val === 'th' || val === 'en') return val;
    }
  } catch {
    // storage blocked or getter throws
  }
  return 'th';
}

function syncHtmlLang(l: Lang): void {
  try {
    if (typeof document !== 'undefined' && document.documentElement) {
      document.documentElement.lang = l;
    }
  } catch {
    // ignore
  }
}

const initialLang = readStorage();
const [currentLang, setCurrentLang] = createSignal<Lang>(initialLang);
syncHtmlLang(initialLang);

export const lang: Accessor<Lang> = currentLang;

export const locale: Accessor<Locale> = () => (currentLang() === 'th' ? 'th-TH' : 'en-GB');

export function setLang(l: Lang): void {
  setCurrentLang(l);
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
  const l = currentLang();
  const table = l === 'th' ? th : en;
  const text = table[key] ?? key;
  return fill(text, params);
}

export function errorText(code: string, message: string): string {
  const key = `error.${code}`;
  if (Object.prototype.hasOwnProperty.call(th, key)) {
    return t(key as Key);
  }
  return message;
}

export interface ScreenTables {
  th: Record<string, string>;
  en: Record<string, string>;
}

function extractPlaceholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]!).sort();
}

export function collectScreens(
  glob: Record<string, unknown>,
): ScreenTables {
  const result: ScreenTables = { th: {}, en: {} };

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

    const m = mod as { th?: Record<string, string>; en?: Record<string, string> } | null | undefined;
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
    const l = currentLang();
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
  T extends { readonly th: Record<string, string>; readonly en: Record<string, string> },
>(
  screen: string,
  table: T,
): (key: keyof T['th'] & string, params?: Params) => string {
  return (key: keyof T['th'] & string, params?: Params): string => {
    const l = currentLang();
    const namespacedKey = `${screen}.${String(key)}`;
    const collectedTable = l === 'th' ? screenLookup.th : screenLookup.en;
    if (Object.prototype.hasOwnProperty.call(collectedTable, namespacedKey)) {
      return fill(collectedTable[namespacedKey], params);
    }
    const currentTable = l === 'th' ? table.th : table.en;
    if (currentTable && Object.prototype.hasOwnProperty.call(currentTable, key)) {
      return fill((currentTable as Record<string, string>)[key], params);
    }
    return namespacedKey;
  };
}
