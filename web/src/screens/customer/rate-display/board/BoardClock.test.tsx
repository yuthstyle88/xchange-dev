// RB-5 tests (feat/fe-rateboard; rate-display.md §9.2, §9.6): BoardClock and BoardHeader.
// Behaviour table rows:
// 1. now = 2026-10-19T12:56:00Z -> 19:56, วันจันทร์ที่ 19 ตุลาคม 2569
// 2. the minute changes -> the time text changes
// 3. within the same minute -> the time nodes are not rebuilt
// 4. header -> brand and sub-brand text
// 5. unmount -> the interval is cleared

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { BOARD_TEXT } from './board-texts';
import { BoardClock } from './BoardClock';
import { BoardHeader } from './BoardHeader';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  vi.useFakeTimers();
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
  vi.useRealTimers();
});

describe('BoardClock & BoardHeader (RB-5)', () => {
  it('now = 2026-10-19T12:56:00Z: renders 19:56 and วันจันทร์ที่ 19 ตุลาคม 2569', () => {
    const fixed = new Date('2026-10-19T12:56:00Z');
    dispose = render(() => <BoardClock now={() => fixed} />, host);
    const time = host.querySelector('.xc-rb-time');
    const date = host.querySelector('.xc-rb-date');
    const colon = host.querySelector('.xc-rb-colon');
    expect(time?.textContent).toBe('19:56');
    expect(colon?.textContent).toBe(':');
    expect(date?.textContent).toBe('วันจันทร์ที่ 19 ตุลาคม 2569');
  });

  it('the minute changes: the time text changes', () => {
    let current = new Date('2026-10-19T12:56:00Z');
    dispose = render(() => <BoardClock now={() => current} />, host);
    expect(host.querySelector('.xc-rb-time')?.textContent).toBe('19:56');

    current = new Date('2026-10-19T12:57:00Z');
    vi.advanceTimersByTime(1000);
    expect(host.querySelector('.xc-rb-time')?.textContent).toBe('19:57');
  });

  it('within the same minute: the time nodes are not rebuilt', () => {
    let current = new Date('2026-10-19T12:56:00Z');
    dispose = render(() => <BoardClock now={() => current} />, host);
    const timeNode = host.querySelector('.xc-rb-time');
    const colonNode = host.querySelector('.xc-rb-colon');
    expect(timeNode).not.toBeNull();
    expect(colonNode).not.toBeNull();

    // Advance 30 seconds (still 19:56 in Bangkok)
    current = new Date('2026-10-19T12:56:30Z');
    vi.advanceTimersByTime(30000);

    expect(host.querySelector('.xc-rb-time')).toBe(timeNode);
    expect(host.querySelector('.xc-rb-colon')).toBe(colonNode);
    expect(host.querySelector('.xc-rb-time')?.textContent).toBe('19:56');
  });

  it('header: brand and sub-brand text', () => {
    dispose = render(() => <BoardHeader />, host);
    const brand = host.querySelector('.xc-rb-brand strong');
    const sub = host.querySelector('.xc-rb-brand small');
    expect(brand?.textContent).toBe(BOARD_TEXT.brand);
    expect(sub?.textContent).toBe(BOARD_TEXT.brandSub);
  });

  it('unmount: the interval is cleared', () => {
    const clearIntervalSpy = vi.spyOn(window, 'clearInterval');
    dispose = render(() => <BoardClock />, host);
    dispose?.();
    dispose = undefined;
    expect(clearIntervalSpy).toHaveBeenCalled();
    clearIntervalSpy.mockRestore();
  });
});
