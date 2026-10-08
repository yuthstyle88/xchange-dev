// V2-parts (#476): the hash query a menu reads and the hand-off that sets it (ticket §7 `hash.ts`,
// design §12.14 *PENDING hand-off*). Written before the implementation by the lane controller
// (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//
// v2's status strip opens `#transactions` with the status PENDING, search and type empty and the
// whole period (`admin-workspace.js:157`); v3 writes that as `#transactions?status=PENDING&from=&to=`
// through `openMenu`, and the menu reads it back through `hashParams()` — the part of the hash after
// `?`. Empty values are kept (the shell reads `from=` / `to=` as "all dates").
import { afterEach, describe, expect, it } from 'vitest';
import { hashParams, openMenu } from './hash';

afterEach(() => {
  location.hash = '';
});

describe('hashParams / openMenu (#476, §12.14)', () => {
  it('openMenu writes id?params into the hash, keeping empty values', () => {
    openMenu('transactions', { status: 'PENDING', from: '', to: '' });
    expect(location.hash).toBe('#transactions?status=PENDING&from=&to=');
  });

  it('hashParams reads the part after ? as query parameters', () => {
    location.hash = '#transactions?status=PENDING&from=&to=';
    const params = hashParams();
    expect(params).toBeInstanceOf(URLSearchParams);
    expect(params.get('status')).toBe('PENDING');
    expect(params.get('from')).toBe('');
    expect(params.has('to')).toBe(true);
    expect(params.get('q')).toBeNull();
  });

  it('a hash with no ? has no parameters, nor has an empty hash', () => {
    location.hash = '#cash';
    expect([...hashParams().keys()]).toEqual([]);
    location.hash = '';
    expect([...hashParams().keys()]).toEqual([]);
  });

  it('what openMenu writes, hashParams reads back, whatever the characters', () => {
    openMenu('transactions', { q: 'a&b c=d', status: 'PENDING', type: '' });
    const params = hashParams();
    expect(params.get('q')).toBe('a&b c=d');
    expect(params.get('status')).toBe('PENDING');
    expect(params.get('type')).toBe('');
    expect(location.hash.startsWith('#transactions?')).toBe(true);
  });
});
