// Specification tests for QU-6 (docs/design/blueprint/queue-tickets.md §4.5, §9.1 row QU-6):
// (c) admin QueueBar reads queue_board, never kiosk_notifications or kiosk_queue.
//
// Written before implementation by test-writer (gemini/studio-11, ordered by
// claude/5edf7bb5, Owner's Rule A). Do not edit, delete or weaken.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { QueueBar } from './QueueBar';

describe('QueueBar reads queue_board (QU-6 item c)', () => {
  let host: HTMLDivElement;
  const disposers: Array<() => void> = [];
  const requestedUrls: string[] = [];

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    requestedUrls.length = 0;

    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : (input as Request).url;
        requestedUrls.push(url);
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: { waiting: [], called: [], myCounter: null },
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }),
    );
  });

  afterEach(() => {
    while (disposers.length) disposers.pop()!();
    host.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
  });

  it('reads queue_board and never kiosk_notifications or kiosk_queue', async () => {
    const [branch] = createSignal('B1');
    const [label] = createSignal('Silom');

    disposers.push(
      render(() => <QueueBar branch={branch} branchLabel={label} />, host),
    );

    for (let i = 0; i < 30; i += 1) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 50));

    const boardCall = requestedUrls.some((u) => u.includes('/api/v3/views/queue_board'));
    const notifCall = requestedUrls.some((u) => u.includes('/api/v3/views/kiosk_notifications'));
    const queueCall = requestedUrls.some((u) => u.includes('/api/v3/views/kiosk_queue'));

    expect(boardCall, `expected call to queue_board, got: ${requestedUrls.join(', ')}`).toBe(true);
    expect(notifCall, `unexpected call to kiosk_notifications, got: ${requestedUrls.join(', ')}`).toBe(false);
    expect(queueCall, `unexpected call to kiosk_queue, got: ${requestedUrls.join(', ')}`).toBe(false);
  });
});
