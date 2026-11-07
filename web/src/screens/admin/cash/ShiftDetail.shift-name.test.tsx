// F55 (docs/design/backdated-keying.md §16.7), restated by the ticket author claude/3b81c9d5: dates are dd/mm/yyyy HH:mm
// (CE, Bangkok) from the shared formatter. A shift's business date is a day, so the label reads `B · 04/11/2026`.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { makeAdminShiftRow } from '../../../shared/testing/fixtures';
import { Dialog, closeDialog } from '../parts/Dialog';
import { openShiftDetail } from './ShiftDetail';

describe('ShiftDetail shift name and business date (CS-16)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        new Response(JSON.stringify({ status: 200, ok: true, data: {} }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
      ),
    );
    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(() => <Dialog />, root);
  });

  afterEach(() => {
    closeDialog();
    dispose?.();
    root.remove();
    vi.unstubAllGlobals();
  });

  it('U6: a row with shiftName "B" and businessDate "2026-11-04" shows the label in shift detail', () => {
    const row = makeAdminShiftRow({
      id: 'S-0001',
      staff: 'EMP-007',
      shiftName: 'B',
      businessDate: '2026-11-04',
    });
    openShiftDetail(row, 'MAIN');

    const meta = document.querySelector('.shift-detail-meta');
    expect(meta).not.toBeNull();
    expect(meta?.textContent).toContain('EMP-007');
    expect(meta?.textContent).toContain('B · 04/11/2026');

    const label = document.querySelector('.shift-label');
    expect(label).not.toBeNull();
    expect(label?.textContent?.trim()).toBe('B · 04/11/2026');
  });

  it('U6b: a row with shiftName null shows no label in shift detail', () => {
    const row = makeAdminShiftRow({
      id: 'S-0002',
      staff: 'EMP-002',
      shiftName: null,
      businessDate: '2026-11-04',
    });
    openShiftDetail(row, 'MAIN');

    const meta = document.querySelector('.shift-detail-meta');
    expect(meta).not.toBeNull();
    expect(meta?.textContent?.trim()).toBe('S-0002 · EMP-002');

    const label = document.querySelector('.shift-label');
    expect(label).toBeNull();
  });
});
