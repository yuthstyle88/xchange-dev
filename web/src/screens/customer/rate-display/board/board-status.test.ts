// RB-2 up-front tests (feat/fe-rateboard skeleton; rate-display.md §9.5, §9.6): synced / delayed / offline, the 30 s
// hide, and the empty panel's text. The implementer makes these pass and must not edit them.
import { describe, expect, it } from 'vitest';
import { BOARD_TEXT } from './board-texts';
import { boardStatus, HIDE_AFTER_MS, type BoardStatusInput } from './board-status';

const NOW = 1_000_000;
const input = (over: Partial<BoardStatusInput>): BoardStatusInput => ({
  pollerState: 'ready',
  lastOkAt: NOW - 1_000,
  now: NOW,
  lineCount: 3,
  ...over,
});

describe('boardStatus (RB-2)', () => {
  it('the hide threshold is 30 s', () => {
    expect(HIDE_AFTER_MS).toBe(30_000);
  });

  it('first load pending (no successful update yet): offline, rates hidden, the loading text', () => {
    expect(boardStatus(input({ pollerState: 'loading', lastOkAt: undefined, lineCount: undefined }))).toEqual({
      status: 'offline',
      ratesVisible: false,
      emptyMessage: BOARD_TEXT.loading,
    });
  });

  it('a recent success with lines: synced, rates shown, no empty text', () => {
    expect(boardStatus(input({}))).toEqual({ status: 'synced', ratesVisible: true, emptyMessage: null });
  });

  it('a 304 counts as a success: state ready with a fresh lastOkAt is synced', () => {
    expect(boardStatus(input({ lastOkAt: NOW - 9_000 })).status).toBe('synced');
  });

  it('the last poll failed but the last success is within 30 s: delayed, rates still shown', () => {
    expect(boardStatus(input({ pollerState: 'error', lastOkAt: NOW - 10_000 }))).toEqual({
      status: 'delayed',
      ratesVisible: true,
      emptyMessage: null,
    });
  });

  it('exactly 30 s since the last success is still shown; just over 30 s is offline', () => {
    expect(boardStatus(input({ pollerState: 'error', lastOkAt: NOW - HIDE_AFTER_MS })).ratesVisible).toBe(true);
    expect(boardStatus(input({ pollerState: 'error', lastOkAt: NOW - HIDE_AFTER_MS - 1 }))).toEqual({
      status: 'offline',
      ratesVisible: false,
      emptyMessage: BOARD_TEXT.askCounter,
    });
  });

  it('no success for over 30 s while the poller still says ready (no poll ran): offline too', () => {
    expect(boardStatus(input({ pollerState: 'ready', lastOkAt: NOW - 45_000 })).status).toBe('offline');
  });

  it('the next success after offline: synced again', () => {
    expect(boardStatus(input({ pollerState: 'ready', lastOkAt: NOW })).status).toBe('synced');
  });

  it('a successful answer with no lines: synced, and the "no currencies" text', () => {
    expect(boardStatus(input({ lineCount: 0 }))).toEqual({
      status: 'synced',
      ratesVisible: true,
      emptyMessage: BOARD_TEXT.noCurrencies,
    });
  });

  it('no status text ever says live or real time (B8, #916)', () => {
    const all = [BOARD_TEXT.synced, BOARD_TEXT.delayed, BOARD_TEXT.offline].flatMap((t) => [t.th, t.en]).join(' ');
    expect(all).not.toMatch(/live|real ?time|เรียลไทม์|实时/i);
  });
});
