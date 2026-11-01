import type { DeskRate } from '../../shared/api/generated/desk_config';
import type { DeskType } from './desk-store';

export function publishedCodes(rates: readonly DeskRate[], type: DeskType): string[] {
  return rates.filter((r) => (type === 'BUY' ? r.buy : r.sell) !== null).map((r) => r.code);
}
