// V2-payments (#470): translation and date helpers

import { locale } from '../../../shared/i18n';

export function formatDate(iso: string): string {
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}
