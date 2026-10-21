// Desk passport read via the v3 server (`POST /api/v3/compute/passport_read`, design recognize-passport-read.md §4).
import { ApiError, type ApiClient } from '../../../shared/api/client';
import type { PassportReadAnswer } from '../../../shared/api/generated/passport_read';
import { newKey } from '../save';

const READ_TIMEOUT_MS = 120_000;

export type ReadFailure =
  | 'aborted'
  | 'timeout'
  | 'failed'
  | 'busy'
  | 'notConfigured'
  | 'imageTooLarge'
  | 'imageInvalid'
  | 'notAllowed';

export class PassportReadError extends Error {
  constructor(readonly kind: ReadFailure) {
    super(kind);
    this.name = 'PassportReadError';
  }
}

function failureOf(err: unknown): ReadFailure {
  if (err instanceof ApiError) {
    switch (err.code) {
      case 'passportReaderTimeout':
        return 'timeout';
      case 'passportReaderBusy':
        return 'busy';
      case 'passportReaderNotConfigured':
        return 'notConfigured';
      case 'imageTooLarge':
        return 'imageTooLarge';
      case 'imageInvalid':
      case 'pdfNotAccepted':
      case 'imageTypeNotAccepted':
        return 'imageInvalid';
      case 'roleNotAllowedHere':
      case 'signIn':
        return 'notAllowed';
      default:
        return 'failed';
    }
  }
  return 'failed';
}

export async function readPassportOnServer(
  api: ApiClient,
  image: Blob,
  opts: { branch: string; signal: AbortSignal },
): Promise<PassportReadAnswer> {
  if (opts.signal.aborted) {
    throw new PassportReadError('aborted');
  }

  let timer: ReturnType<typeof setTimeout> | undefined;
  let onAbort: (() => void) | undefined;

  const race = new Promise<never>((_resolve, reject) => {
    onAbort = () => reject(new PassportReadError('aborted'));
    opts.signal.addEventListener('abort', onAbort, { once: true });
    timer = setTimeout(() => reject(new PassportReadError('timeout')), READ_TIMEOUT_MS);
  });

  try {
    const path = `/api/v3/compute/passport_read?branch=${encodeURIComponent(opts.branch)}`;
    return await Promise.race([
      api.postBytes<PassportReadAnswer>(
        path,
        image,
        image.type || 'image/jpeg',
        newKey(),
        opts.signal,
      ),
      race,
    ]);
  } catch (err) {
    if (err instanceof PassportReadError) throw err;
    if (opts.signal.aborted) throw new PassportReadError('aborted');
    throw new PassportReadError(failureOf(err));
  } finally {
    if (timer !== undefined) clearTimeout(timer);
    if (onAbort !== undefined) opts.signal.removeEventListener('abort', onAbort);
  }
}
