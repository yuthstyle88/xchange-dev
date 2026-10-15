import { ApiError } from '../../shared/api/client';
import { wordedError } from '../../shared/i18n/worded-error';

export function refusalText(err: unknown, t: (key: string) => string): string {
  if (err instanceof ApiError) {
    if (err.status === 400 && err.code === 'invalidRequest' && err.field === undefined) {
      return t('documents.badBranch');
    }
    if (err.status < 500 && err.message) {
      return err.message;
    }
  }
  return wordedError(err);
}
