// TD-3 #1291: the screen's address (design §3 `TransactionRoute`, D1). Written before the implementation by the
// up-front test writer (claude/709ec749). Do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';
import { branchFromSearch, idFromPath } from './route';

const ID = '7f0c2b1e-4a5d-4c3b-9e8f-0a1b2c3d4e5f';

describe('idFromPath', () => {
  it('reads the one segment after /desk/transactions/', () => {
    expect(idFromPath(`/desk/transactions/${ID}`)).toBe(ID);
  });

  it('is null for any other path', () => {
    for (const p of ['/desk/transactions', '/desk/transactions/', `/desk/transactions/${ID}/x`, '/desk', `/x/desk/transactions/${ID}`]) {
      expect(idFromPath(p), p).toBeNull();
    }
  });
});

describe('branchFromSearch', () => {
  it("reads an owner's ?branch=", () => {
    expect(branchFromSearch('?branch=B2')).toBe('B2');
    expect(branchFromSearch('?x=1&branch=MAIN')).toBe('MAIN');
  });

  it('is undefined when absent or empty', () => {
    expect(branchFromSearch('')).toBeUndefined();
    expect(branchFromSearch('?branch=')).toBeUndefined();
  });
});
