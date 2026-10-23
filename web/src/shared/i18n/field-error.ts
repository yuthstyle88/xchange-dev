import { ApiError } from '../api/client';
import { t } from './index';
import { th, type Key } from './th';
import { isServerFailure, wordedError } from './worded-error';

export interface FieldMessage {
  field?: string;
  message: string;
}

export function fieldError(
  err: unknown,
  labels: Readonly<Record<string, string>>,
): FieldMessage {
  if (!(err instanceof ApiError) || isServerFailure(err)) {
    return { message: wordedError(err) };
  }
  const hasField =
    err.field !== undefined && Object.prototype.hasOwnProperty.call(labels, err.field);
  const codeKey = `input.code.${err.code}` as Key;
  if (Object.prototype.hasOwnProperty.call(th, codeKey)) {
    return hasField
      ? { field: err.field, message: t(codeKey) }
      : { message: t(codeKey) };
  }
  if (err.code === 'validationFailed') {
    return hasField
      ? { field: err.field, message: t('input.invalid', { field: labels[err.field!] }) }
      : { message: t('input.invalidNoField') };
  }
  return hasField
    ? { field: err.field, message: wordedError(err) }
    : { message: wordedError(err) };
}
