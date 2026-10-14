// Written before the implementation by the test writer gemini/warut-4 (ticket author claude/e92db6), against design D9a (PR 1656). Do not edit, delete or weaken.
//
// Shared QrCode component tests (#1374, D9a in PR 1656).
// Contract:
// - `QrCode(props: { value: string; label: string })` renders an inline <svg>
//   element client-side via `qrcode-generator` for any `value`.
// - The rendered output carries its accessible `label` (e.g. via aria-label or title).
// - Calls `qrcode-generator`'s `addData` with the exact `value` string.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { QrCode } from './QrCode';

const { mockAddData, mockMake, mockCreateSvgTag, mockFactory } = vi.hoisted(() => {
  const mockAddData = vi.fn();
  const mockMake = vi.fn();
  const mockCreateSvgTag = vi.fn(
    (_opts?: unknown) =>
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100"/></svg>',
  );
  const mockFactory = vi.fn((_typeNumber?: number, _errorCorrectionLevel?: string) => ({
    addData: mockAddData,
    make: mockMake,
    createSvgTag: mockCreateSvgTag,
  }));
  return { mockAddData, mockMake, mockCreateSvgTag, mockFactory };
});

vi.mock('qrcode-generator', () => ({
  default: mockFactory,
  qrcode: mockFactory,
  __esModule: true,
}));

let dispose: (() => void) | undefined;

function mount(ui: () => JSX.Element): HTMLElement {
  const container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(ui, container);
  return container;
}

beforeEach(() => {
  mockAddData.mockClear();
  mockMake.mockClear();
  mockCreateSvgTag.mockClear();
  mockFactory.mockClear();
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
});

describe('shared QrCode component (D9a in PR 1656, #1374)', () => {
  it('1. renders inline <svg> element for the given value', () => {
    const container = mount(() => <QrCode value="https://108.plaza/app/book?branch=XC-B01" label="Booking QR" />);
    const svg = container.querySelector('svg');
    expect(svg, 'QrCode must render an inline <svg> element').not.toBeNull();
  });

  it('2. passes the exact value to qrcode-generator addData and makes the code', () => {
    const targetUrl = 'https://108.plaza/app/book?branch=XC-B01';
    mount(() => <QrCode value={targetUrl} label="Booking QR" />);

    expect(mockAddData, 'addData must be called with the target URL').toHaveBeenCalledWith(targetUrl);
    expect(mockMake, 'make must be called to generate the QR matrix').toHaveBeenCalled();
  });

  it('3. carries its accessible label', () => {
    const labelText = 'QR code for branch booking';
    const container = mount(() => (
      <QrCode value="https://108.plaza/app/book?branch=XC-B01" label={labelText} />
    ));

    const labeled =
      container.querySelector(`[aria-label="${labelText}"]`) ??
      [...container.querySelectorAll('title')].find((el) => el.textContent === labelText);

    expect(
      labeled,
      `rendered QR code element or wrapper must carry the accessible label "${labelText}"`,
    ).toBeTruthy();
  });

  it('4. renders for any arbitrary value (encoder input matches prop)', () => {
    const arbitraryValue = 'https://custom.domain.example/booking/v1?token=xyz123&lang=th';
    mount(() => <QrCode value={arbitraryValue} label="Arbitrary Link" />);

    expect(mockAddData, 'addData must receive arbitrary value').toHaveBeenCalledWith(arbitraryValue);
  });

  it('5. reacts to dynamic updates of the value prop', () => {
    const [url, setUrl] = createSignal('https://initial.test');
    mount(() => <QrCode value={url()} label="Dynamic QR" />);

    expect(mockAddData).toHaveBeenCalledWith('https://initial.test');

    setUrl('https://updated.test');
    expect(mockAddData).toHaveBeenCalledWith('https://updated.test');
  });
});
