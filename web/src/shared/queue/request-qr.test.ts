// QW-1 (kiosk-shared-phone §5r F13, row R1): one parser for the customer's request QR `<origin>/q#<token>` (§4 D3),
// shared by the desk scan, the /app/queue page and the /q page. Written before the implementation by the
// sub-controller claude/36d9dc6a. Do not edit, delete or weaken these tests; add tests in a file of your own.
//
// Pinned: `web/src/shared/queue/request-qr.ts` exports `parseRequestQr(raw: string): string | null`.
// It answers the token for a URL whose path is exactly `/q` and whose fragment is non-empty (any origin, as
// scanned), trimming surrounding whitespace; anything else answers null.
import { describe, expect, it } from 'vitest';
import { parseRequestQr } from './request-qr';

describe('QW-1 R1: parseRequestQr', () => {
  it.each([
    ['https://xc-v3.spacing.cloud/q#b1f0c5e2-3a4d-4c8e-9f10-2a3b4c5d6e7f', 'b1f0c5e2-3a4d-4c8e-9f10-2a3b4c5d6e7f'],
    ['http://localhost:8080/q#tok', 'tok'],
    ['  https://h/q#tok \n', 'tok'],
  ])('%s -> the token', (raw, token) => {
    expect(parseRequestQr(raw)).toBe(token);
  });

  it.each([
    ['https://h/q#', 'empty fragment'],
    ['https://h/q', 'no fragment'],
    ['https://h/qq#tok', 'another path'],
    ['https://h/app/q#tok', 'another path'],
    ['https://h/c#v1.webRequest.k1.1.tok.mac', 'the /c copy link'],
    ['test-qr-token-xyz', 'a booking QR token (bare)'],
    ['', 'empty'],
    ['not a url at all', 'garbage'],
  ])('%s (%s) -> null', (raw) => {
    expect(parseRequestQr(raw)).toBeNull();
  });
});
