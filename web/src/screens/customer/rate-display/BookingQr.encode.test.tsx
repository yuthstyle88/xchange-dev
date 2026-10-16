import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setAudience, setLang } from '../../../shared/i18n';
import { BookingQr } from './BookingQr';

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

describe('Rate board BookingQr percent-encoding (#2370)', () => {
  let root: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    mockAddData.mockClear();
    mockMake.mockClear();
    mockCreateSvgTag.mockClear();
    mockFactory.mockClear();
    setAudience('customer');
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root.remove();
    document.body.innerHTML = '';
    setLang('th');
    setAudience('staff');
  });

  it('row 1: Branch A&B -> data-qr-url and the QR data = <origin>/app/book?branch=A%26B', () => {
    dispose = render(() => <BookingQr branchCode="A&B" bookingEnabled={true} />, root);

    const qrEl = root.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qrEl).not.toBeNull();

    const expectedUrl = `${window.location.origin}/app/book?branch=A%26B`;
    expect(qrEl!.getAttribute('data-qr-url')).toBe(expectedUrl);
    expect(mockAddData).toHaveBeenCalledWith(expectedUrl);
  });

  it('row 2: Branch B#2 -> ?branch=B%232', () => {
    dispose = render(() => <BookingQr branchCode="B#2" bookingEnabled={true} />, root);

    const qrEl = root.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qrEl).not.toBeNull();

    const expectedUrl = `${window.location.origin}/app/book?branch=B%232`;
    expect(qrEl!.getAttribute('data-qr-url')).toBe(expectedUrl);
    expect(mockAddData).toHaveBeenCalledWith(expectedUrl);
  });

  it('row 3: Branch สีลม 1 -> ?branch=%E0%B8%AA%E0%B8%B5%E0%B8%A5%E0%B8%A1%201; the QR data is ASCII-only', () => {
    dispose = render(() => <BookingQr branchCode="สีลม 1" bookingEnabled={true} />, root);

    const qrEl = root.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qrEl).not.toBeNull();

    const expectedUrl = `${window.location.origin}/app/book?branch=%E0%B8%AA%E0%B8%B5%E0%B8%A5%E0%B8%A1%201`;
    expect(qrEl!.getAttribute('data-qr-url')).toBe(expectedUrl);
    expect(mockAddData).toHaveBeenCalledWith(expectedUrl);

    const qrData = String(mockAddData.mock.calls[0]?.[0] ?? '');
    expect(/^[\x00-\x7F]+$/.test(qrData)).toBe(true);
  });

  it('row 4: Branch XC-B01 -> Unchanged ?branch=XC-B01 (kept, BookingQr.test.tsx:185-206)', () => {
    const [code, setCode] = createSignal('XC-B01');
    dispose = render(() => <BookingQr branchCode={code()} bookingEnabled={true} />, root);

    const qrEl = root.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qrEl).not.toBeNull();

    const expectedPlain = `${window.location.origin}/app/book?branch=XC-B01`;
    expect(qrEl!.getAttribute('data-qr-url')).toBe(expectedPlain);
    expect(mockAddData).toHaveBeenLastCalledWith(expectedPlain);

    // Updating to a code with reserved chars encodes it, and switching back to XC-B01 keeps XC-B01 unchanged
    setCode('XC-B01&2');
    expect(qrEl!.getAttribute('data-qr-url')).toBe(
      `${window.location.origin}/app/book?branch=XC-B01%262`,
    );

    setCode('XC-B01');
    expect(qrEl!.getAttribute('data-qr-url')).toBe(expectedPlain);
  });

  it('row 5: Disabled or missing branch -> Renders nothing (kept, BookingQr.test.tsx:208-230)', () => {
    const [enabled, setEnabled] = createSignal(false);
    const [code, setCode] = createSignal<string | undefined>('A&B');

    dispose = render(() => <BookingQr branchCode={code()} bookingEnabled={enabled()} />, root);

    // Disabled branch renders nothing
    expect(root.querySelector('[data-booking-qr]')).toBeNull();
    expect(mockAddData).not.toHaveBeenCalled();

    // Missing branch code renders nothing even when bookingEnabled is true
    setCode(undefined);
    setEnabled(true);
    expect(root.querySelector('[data-booking-qr]')).toBeNull();
    expect(mockAddData).not.toHaveBeenCalled();

    // Providing a branch code while enabled renders the encoded QR, and disabling hides it again
    setCode('A&B');
    const qrEl = root.querySelector<HTMLElement>('[data-booking-qr]');
    expect(qrEl).not.toBeNull();
    expect(qrEl!.getAttribute('data-qr-url')).toBe(
      `${window.location.origin}/app/book?branch=A%26B`,
    );

    setEnabled(false);
    expect(root.querySelector('[data-booking-qr]')).toBeNull();
  });

  it('edge case 1 (§9): The booking page decodes the code back through URLSearchParams, so encoding round-trips', () => {
    const [code, setCode] = createSignal('A&B');
    dispose = render(() => <BookingQr branchCode={code()} bookingEnabled={true} />, root);

    for (const sample of ['A&B', 'B#2', 'สีลม 1', 'XC-B01']) {
      setCode(sample);
      const qrEl = root.querySelector<HTMLElement>('[data-booking-qr]');
      expect(qrEl).not.toBeNull();
      const rawUrl = qrEl!.getAttribute('data-qr-url')!;
      const parsed = new URL(rawUrl);
      expect(parsed.pathname).toBe('/app/book');
      expect(parsed.hash).toBe('');
      expect(parsed.searchParams.get('branch')).toBe(sample);
    }
  });

  it('edge case 2 (§9): The caption shows the same encoded path as the QR', () => {
    const [code, setCode] = createSignal('A&B');
    dispose = render(() => <BookingQr branchCode={code()} bookingEnabled={true} />, root);

    for (const sample of ['A&B', 'B#2', 'สีลม 1', 'XC-B01']) {
      setCode(sample);
      const qrEl = root.querySelector<HTMLElement>('[data-booking-qr]');
      const pathEl = root.querySelector<HTMLElement>('.xc-rate-display-booking-qr-path');
      expect(qrEl).not.toBeNull();
      expect(pathEl).not.toBeNull();

      const expectedPath = `/app/book?branch=${encodeURIComponent(sample)}`;
      expect(pathEl!.textContent).toBe(expectedPath);
      expect(qrEl!.getAttribute('data-qr-url')).toBe(`${window.location.origin}${expectedPath}`);
    }
  });
});
