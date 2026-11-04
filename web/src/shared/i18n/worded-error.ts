import { ApiError } from '../api/client';
import { errorText, t } from './index';
import type { Key } from './th';

const PLACE_KEY: Record<string, Key> = {
  DRAWER: 'error.stockPlaceDrawer',
  SAFE: 'error.stockPlaceSafe',
  POSITION: 'error.stockPlacePosition',
};

function stockDetail(details: unknown): string | undefined {
  if (!details || typeof details !== 'object') return undefined;
  const lines = (details as { lines?: unknown }).lines;
  if (!Array.isArray(lines) || lines.length === 0) return undefined;
  const items: string[] = [];
  for (const line of lines) {
    if (!line || typeof line !== 'object') return undefined;
    const { currency, place, shortBy } = line as Record<string, unknown>;
    if (
      typeof currency !== 'string' ||
      !currency ||
      typeof shortBy !== 'string' ||
      !shortBy ||
      typeof place !== 'string' ||
      !Object.prototype.hasOwnProperty.call(PLACE_KEY, place)
    ) {
      return undefined;
    }
    items.push(t('error.stockShortItem', { currency, place: t(PLACE_KEY[place]!), shortBy }));
  }
  return t('error.stockInsufficientDetail', { items: items.join(', ') });
}

export function isServerFailure(err: unknown): boolean {
  return (
    err instanceof ApiError &&
    (err.status >= 500 || err.code === 'serverError' || err.code === 'httpError')
  );
}

export function wordedError(err: unknown): string {
  if (err instanceof ApiError) {
    if (isServerFailure(err)) {
      return t('error.serverError');
    }
    if (err.code === 'stockInsufficient') {
      const detail = stockDetail(err.details);
      if (detail) return detail;
    }
    const text = errorText(err.code, err.message);
    if (!text || !text.trim() || text === err.code) {
      return t('error.serverError');
    }
    return text;
  }
  return t('error.network');
}

