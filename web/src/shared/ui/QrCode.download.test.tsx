import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { QrCode } from './QrCode';

describe('QrCode download button (#4327)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | undefined;
  const originalCreateObjectURL = URL.createObjectURL;
  const originalRevokeObjectURL = URL.revokeObjectURL;

  function mount(ui: () => JSX.Element): HTMLDivElement {
    container = document.createElement('div');
    document.body.appendChild(container);
    dispose = render(ui, container);
    return container;
  }

  beforeEach(() => {
    document.body.innerHTML = '';
    vi.stubGlobal(
      'Image',
      class {
        onload: (() => void) | null = null;
        onerror: (() => void) | null = null;
        set src(_v: string) {
          this.onload?.();
        }
      },
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    URL.createObjectURL = originalCreateObjectURL;
    URL.revokeObjectURL = originalRevokeObjectURL;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('renders a button labelled downloadLabel when downloadName is set and downloads <downloadName>.png via canvas toBlob', () => {
    const drawImage = vi.fn();
    const fillRect = vi.fn();
    const getContextSpy = vi
      .spyOn(HTMLCanvasElement.prototype, 'getContext')
      .mockReturnValue({ drawImage, fillRect, fillStyle: '' } as unknown as CanvasRenderingContext2D);
    const pngBlob = new Blob(['png-bytes'], { type: 'image/png' });
    const toBlobSpy = vi
      .spyOn(HTMLCanvasElement.prototype, 'toBlob')
      .mockImplementation((cb: BlobCallback) => cb(pngBlob));
    URL.createObjectURL = vi.fn(() => 'blob:http://xc.test/qr-png') as unknown as typeof URL.createObjectURL;
    URL.revokeObjectURL = vi.fn() as unknown as typeof URL.revokeObjectURL;

    let clickedDownload = '';
    let clickedHref = '';
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      clickedDownload = this.download;
      clickedHref = this.href;
    });

    const el = mount(() => (
      <QrCode
        value="qr-token-123"
        label="Check-in QR"
        downloadName="booking-B261021-0001"
        downloadLabel="Download QR"
      />
    ));

    const btn = el.querySelector<HTMLButtonElement>('button');
    expect(btn).not.toBeNull();
    expect(btn!.textContent).toBe('Download QR');

    btn!.click();

    expect(getContextSpy).toHaveBeenCalledWith('2d');
    expect(drawImage).toHaveBeenCalled();
    expect(toBlobSpy).toHaveBeenCalledWith(expect.any(Function), 'image/png');
    expect(URL.createObjectURL).toHaveBeenCalledWith(pngBlob);
    expect(clickSpy).toHaveBeenCalledTimes(1);
    expect(clickedDownload).toBe('booking-B261021-0001.png');
    expect(clickedHref).toBe('blob:http://xc.test/qr-png');
  });

  it('calls neither toBlob nor HTMLAnchorElement.click when Image fails to load (onerror)', () => {
    vi.stubGlobal(
      'Image',
      class {
        onload: (() => void) | null = null;
        onerror: (() => void) | null = null;
        set src(_v: string) {
          this.onerror?.();
        }
      },
    );

    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
      drawImage: vi.fn(),
      fillRect: vi.fn(),
      fillStyle: '',
    } as unknown as CanvasRenderingContext2D);
    const toBlobSpy = vi.spyOn(HTMLCanvasElement.prototype, 'toBlob');
    const clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click');

    const el = mount(() => (
      <QrCode
        value="qr-token-123"
        label="Check-in QR"
        downloadName="booking-B261021-0001"
        downloadLabel="Download QR"
      />
    ));

    const btn = el.querySelector<HTMLButtonElement>('button')!;
    btn.click();

    expect(toBlobSpy).not.toHaveBeenCalled();
    expect(clickSpy).not.toHaveBeenCalled();
  });

  it('renders no button when downloadName is omitted', () => {
    const el = mount(() => <QrCode value="qr-token-123" label="Check-in QR" />);

    expect(el.querySelector('svg')).not.toBeNull();
    expect(el.querySelector('button')).toBeNull();
    expect(el.children.length).toBe(1);
    expect(el.firstElementChild?.getAttribute('role')).toBe('img');
  });
});
