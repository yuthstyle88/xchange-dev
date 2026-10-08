// V2-parts (#476): the page-wide refresh counter (ticket §7 `refresh.ts`, design §12.2, §12.6).
// Written before the implementation by the lane controller (claude/7ca80c). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// No admin menu polls (§12.6): a menu re-reads when `refreshTick()` moves. The shell's `รีเฟรช` and
// a saved write (`pendingWrite`'s success, `pending-write.test.tsx`) move it through
// `requestRefresh()`.
import { createEffect, createRoot } from 'solid-js';
import { describe, expect, it } from 'vitest';
import { refreshTick, requestRefresh } from './refresh';

describe('refreshTick / requestRefresh (#476, §12.6)', () => {
  it('each requestRefresh moves refreshTick by exactly one', () => {
    const start = refreshTick();
    expect(typeof start).toBe('number');
    requestRefresh();
    expect(refreshTick()).toBe(start + 1);
    requestRefresh();
    requestRefresh();
    expect(refreshTick()).toBe(start + 3);
  });

  it('a read that depends on refreshTick runs again on each requestRefresh, and only then', () => {
    const seen: number[] = [];
    const dispose = createRoot((d) => {
      createEffect(() => {
        seen.push(refreshTick());
      });
      return d;
    });
    const first = seen.length;
    expect(first).toBe(1);
    requestRefresh();
    expect(seen.length).toBe(2);
    expect(seen[1]).toBe(seen[0]! + 1);
    requestRefresh();
    expect(seen.length).toBe(3);
    dispose();
    requestRefresh();
    expect(seen.length, 'a disposed reader is not run').toBe(3);
  });
});
