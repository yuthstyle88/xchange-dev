import { ApiError } from '../api/client';
import { errorText, t } from './index';

export function wordedError(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.status >= 500 || err.code === 'serverError' || err.code === 'httpError') {
      return t('error.serverError');
    }
    const text = errorText(err.code, err.message);
    if (!text || !text.trim() || text === err.code) {
      return t('error.serverError');
    }
    return text;
  }
  return t('error.network');
}
