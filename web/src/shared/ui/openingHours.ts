// APP-12 / APP-8 shared opening hours formatter (design customer-app.md §4.5, §9.1 C08).

export const DAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'] as const;
export type DayKey = (typeof DAYS)[number];

export interface DayInterval {
  open: string;
  close: string;
}

export type OpeningHoursData = Record<string, DayInterval[]> | null | undefined;

const DAY_LABELS: Record<'th' | 'en' | 'zh', Record<DayKey, string>> = {
  th: {
    mon: 'จันทร์',
    tue: 'อังคาร',
    wed: 'พุธ',
    thu: 'พฤหัสบดี',
    fri: 'ศุกร์',
    sat: 'เสาร์',
    sun: 'อาทิตย์',
  },
  en: {
    mon: 'Mon',
    tue: 'Tue',
    wed: 'Wed',
    thu: 'Thu',
    fri: 'Fri',
    sat: 'Sat',
    sun: 'Sun',
  },
  zh: {
    mon: '周一',
    tue: '周二',
    wed: '周三',
    thu: '周四',
    fri: '周五',
    sat: '周六',
    sun: '周日',
  },
};

const CLOSED_LABELS: Record<'th' | 'en' | 'zh', string> = {
  th: 'ปิดทำการ',
  en: 'Closed',
  zh: '休息',
};

function normalizeLang(lang?: string): 'th' | 'en' | 'zh' {
  if (!lang) return 'th';
  if (lang === 'zh' || lang.startsWith('zh-') || lang.startsWith('zh_')) return 'zh';
  if (lang === 'en' || lang.startsWith('en-') || lang.startsWith('en_')) return 'en';
  return 'th';
}

/**
 * Pure function formatting opening hours across days in mon..sun order.
 * Missing or empty day entries are shown as closed.
 * A Mon-Tue only branch never reads as open daily.
 */
export function formatOpeningHours(hours: OpeningHoursData, lang?: string): string | null {
  if (!hours || typeof hours !== 'object') {
    return null;
  }

  const normalized = normalizeLang(lang);
  const dayNames = DAY_LABELS[normalized];
  const closed = CLOSED_LABELS[normalized];

  const parts: string[] = [];
  for (const day of DAYS) {
    const dayLabel = dayNames[day];
    const intervals = hours[day];
    if (Array.isArray(intervals) && intervals.length > 0) {
      const valid = intervals.filter((i) => i.open && i.close);
      if (valid.length > 0) {
        parts.push(`${dayLabel}: ${valid.map((i) => `${i.open} - ${i.close}`).join(', ')}`);
        continue;
      }
    }
    parts.push(`${dayLabel}: ${closed}`);
  }

  return parts.join(', ');
}
