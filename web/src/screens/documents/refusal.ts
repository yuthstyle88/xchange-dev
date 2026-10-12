// #1349 V3-documents (design §A.7, [required]): a no-field `400 invalidRequest` from these views can only be a
// malformed branch, so it keeps v2's sentence `สาขาไม่ถูกต้อง`; every other refusal is the view's own sentence.
import type { ApiError } from '../../shared/api/client';

export function refusalText(err: ApiError, t: (key: string) => string): string {
  if (err.status === 400 && err.code === 'invalidRequest' && err.field === undefined) {
    return t('documents.badBranch');
  }
  return err.message;
}
