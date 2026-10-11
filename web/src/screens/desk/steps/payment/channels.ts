// The payment channel rule of v2's desk-payments.js:1-24 (design D-7), over the desk config's `paymentMethods`.
import type { DeskPaymentMethod } from '../../../../shared/api/generated/desk_config';

/** v2's six methods, in v2's order (dh:117). */
export const METHODS = ['Credit Card', 'Alipay', 'PromptPay', 'Cash', 'Bank Transfer', 'SCB Transfer'] as const;

/** The config row a method reads: `SCB Transfer` reads `Bank Transfer`'s (desk-payments.js). */
export function ruleFor(
  rules: readonly DeskPaymentMethod[] | undefined,
  method: string,
): DeskPaymentMethod | undefined {
  const id = method === 'SCB Transfer' ? 'Bank Transfer' : method;
  return rules?.find((r) => r.id === id);
}

/** A method is open when its row is enabled and its flag for the direction is on (BUY → `buy`, SELL → `sell`). */
export function isOpen(rules: readonly DeskPaymentMethod[] | undefined, method: string, type: string): boolean {
  const r = ruleFor(rules, method);
  if (!r || !r.enabled) return false;
  return type === 'SELL' ? r.sell : r.buy;
}
