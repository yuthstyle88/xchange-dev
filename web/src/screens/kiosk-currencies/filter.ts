import type { KioskCurrency } from '../../shared/api/generated/kiosk_currencies';

export function matches(c: KioskCurrency, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) {
    return true;
  }
  const parts = [
    c.code,
    c.name,
    c.nameEn,
    ...c.countries.flatMap((country) => [country.code, country.en, country.th]),
  ];
  return parts.join(' ').toLowerCase().includes(q);
}
