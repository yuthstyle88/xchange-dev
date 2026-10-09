export const LANGS = ['th', 'en', 'zh'] as const;
export type Lang = (typeof LANGS)[number];

export type Audience = 'customer' | 'staff';

export const SPOKEN: Record<Audience, readonly Lang[]> = {
  customer: ['th', 'en', 'zh'],
  staff: ['th', 'en'],
};

export const CUSTOMER_ROOT = 'customer';

export const LOCALE: Record<Lang, 'th-TH' | 'en-GB' | 'zh-CN'> = {
  th: 'th-TH',
  en: 'en-GB',
  zh: 'zh-CN',
};

export const HTML_LANG: Record<Lang, string> = {
  th: 'th',
  en: 'en',
  zh: 'zh-CN',
};

export const NATIVE_NAME: Record<Lang, string> = {
  th: 'ไทย',
  en: 'English',
  zh: '中文',
};
