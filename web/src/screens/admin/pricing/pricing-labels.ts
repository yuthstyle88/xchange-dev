// §9 status / flag labels: TH/EN, V.9's wording (#3766).
import { lang } from '../../../shared/i18n';
import { en, th } from './i18n';
import type { PricingFlagCode, RowStatusCode } from './pricing-board.types';

export function statusLabel(status: RowStatusCode): string {
  const t = lang() === 'en' ? en : th;
  return t[`status.${status}` as keyof typeof th] ?? '';
}

export function flagLabel(flag: PricingFlagCode): string {
  const t = lang() === 'en' ? en : th;
  return t[`flag.${flag}` as keyof typeof th] ?? '';
}
