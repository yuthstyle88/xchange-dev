import { describe, expect, it, vi } from 'vitest';
import type { ApiClient } from '../../../shared/api/client';
import { decideRefusal, decideWrite, type DecidePayload } from './decide';

describe('admin parts: decide', () => {
  it('decideRefusal requires a reason on REJECTED with empty or whitespace reason', () => {
    expect(decideRefusal('REJECTED', '')).toBe('reasonRequired');
    expect(decideRefusal('REJECTED', '   ')).toBe('reasonRequired');
  });

  it('decideRefusal returns null on REJECTED with reason or on APPROVED', () => {
    expect(decideRefusal('REJECTED', 'some reason')).toBeNull();
    expect(decideRefusal('APPROVED', '')).toBeNull();
  });

  it('decideWrite issues one POST to /api/approvals/:id/decide with payload', async () => {
    const commandSpy = vi.fn().mockResolvedValue({ ok: true });
    const api = { command: commandSpy } as unknown as ApiClient;
    const writer = decideWrite(api, 'req-42');
    const payload: DecidePayload = {
      outcome: 'APPROVED',
      reason: 'looks good',
      branch: 'main',
    };

    await writer.send(payload);

    expect(commandSpy).toHaveBeenCalledTimes(1);
    expect(commandSpy).toHaveBeenCalledWith(
      'POST',
      '/api/approvals/req-42/decide',
      payload,
      expect.any(String),
    );
  });
});
