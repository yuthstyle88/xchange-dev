// The desk opened on `?booking=<id>` (design xchange-v3-wave3-desk.md §3.6 B1–B3, D-13, §5.3; #543 Part 3).
// A CONFIRMED booking prefills type, currency and amount through the store's setters and locks them. It never sets the
// rate (D-13: the quote uses the published rate) and makes no booking reference (D-10). The save carries its id.
import { createSignal } from 'solid-js';
import { ApiError, type ApiClient } from '../../../../shared/api/client';
import { screenT } from '../../../../shared/i18n';
import type { DeskEntryState, DeskStore, DeskType } from '../../desk-store';
import type { DeskEntry } from '../../entries';
import * as texts from './i18n';

const t = screenT('desk.entries.booking', texts);

/** v2's booking object (`api/src/presentation/dto/booking.rs` `booking_json` + `customerId`); the fields read here. */
interface BookingAnswer {
  id?: unknown;
  name?: unknown;
  type?: unknown;
  currency?: unknown;
  amount?: unknown;
  status?: unknown;
  expiresAt?: unknown;
}

function text(v: unknown): string {
  return typeof v === 'string' ? v : '';
}

/** The server's amount written as a string, unchanged (W7). */
function amountText(v: unknown): string {
  return typeof v === 'number' || typeof v === 'string' ? String(v) : '';
}

function expiresText(v: unknown): string {
  if (typeof v !== 'string' || v === '') return '';
  const at = new Date(v);
  return Number.isNaN(at.getTime()) ? v : at.toLocaleString('th-TH', { timeZone: 'Asia/Bangkok' });
}

/** A state that refuses the save with `reason` and changes nothing in the store. */
function refusedState(id: string, reason: string): DeskEntryState {
  const [confirmed, setConfirmed] = createSignal(false);
  return {
    kind: 'booking',
    id,
    banner: () => reason,
    locked: false,
    identityConfirmed: confirmed,
    setIdentityConfirmed: (v) => setConfirmed(v),
    payloadExtras: () => ({}),
    refusal: () => reason,
  };
}

export const entry: DeskEntry = {
  param: 'booking',
  async load(store: DeskStore, id: string, api: ApiClient): Promise<DeskEntryState> {
    let b: BookingAnswer;
    try {
      b = await api.post<{ action: 'get'; payload: { branchId: string; id: string } }, BookingAnswer>(
        store.path.bookings,
        { action: 'get', payload: { branchId: store.branch(), id } },
      );
    } catch (e) {
      const refused = e instanceof ApiError && e.status >= 400 && e.status < 500 && e.code !== 'httpError';
      return refusedState(id, refused ? e.message : t('failed'));
    }
    if (!b || text(b.status) !== 'CONFIRMED') {
      return refusedState(id, t('notConfirmed'));
    }

    const name = text(b.name);
    const type = text(b.type);
    if (type === 'BUY' || type === 'SELL') store.setType(type as DeskType);
    const currency = text(b.currency);
    if (currency !== '') store.setCurrency(currency);
    const amount = amountText(b.amount);
    if (amount !== '') store.setAmount(amount);

    const expires = expiresText(b.expiresAt);
    const banner = expires === '' ? t('bannerNoExpiry', { id, name }) : t('banner', { id, name, expires });
    const [confirmed, setConfirmed] = createSignal(false);
    return {
      kind: 'booking',
      id,
      banner: () => banner,
      locked: true,
      identityLabel: t('identityLabel', { name }),
      identityConfirmed: confirmed,
      setIdentityConfirmed: (v) => setConfirmed(v),
      payloadExtras: () => ({ bookingId: id }),
      refusal: () => (confirmed() ? undefined : t('confirmIdentity')),
    };
  },
};
