// F20 #6430: admin detail shows backdated row when backdated is true
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { ApiClient } from '../../../shared/api/client';
import { makeTransactionRow } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { TransactionDetail } from './TransactionDetail';

function fakeApi(): ApiClient {
  return {
    view: vi.fn().mockReturnValue(new Promise(() => {})),
    command: vi.fn(),
    logout: vi.fn(),
  } as unknown as ApiClient;
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  setLang('th');
});

describe('TransactionDetail backdated mark (#6430)', () => {
  it('E4: shows dt "ย้อนหลัง" with dd "ใช่ — คีย์ย้อนหลัง" when backdated is true', () => {
    const r = makeTransactionRow({ id: 'TX-1', backdated: true });

    dispose = render(
      () => <TransactionDetail api={fakeApi()} branch="MAIN" row={r} onClose={() => {}} />,
      host,
    );

    const dts = Array.from(host.querySelectorAll('dt'));
    const backdatedDt = dts.find((dt) => dt.textContent?.trim() === 'ย้อนหลัง');
    expect(backdatedDt).toBeDefined();

    const dd = backdatedDt?.nextElementSibling;
    expect(dd?.tagName.toLowerCase()).toBe('dd');
    expect(dd?.textContent?.trim()).toBe('ใช่ — คีย์ย้อนหลัง');
  });

  it('E5: does not show backdated dt/dd when backdated is false', () => {
    const r = makeTransactionRow({ id: 'TX-2', backdated: false });

    dispose = render(
      () => <TransactionDetail api={fakeApi()} branch="MAIN" row={r} onClose={() => {}} />,
      host,
    );

    const dts = Array.from(host.querySelectorAll('dt'));
    const backdatedDt = dts.find((dt) => dt.textContent?.trim() === 'ย้อนหลัง');
    expect(backdatedDt).toBeUndefined();
    expect(host.textContent).not.toContain('ใช่ — คีย์ย้อนหลัง');
  });
});
