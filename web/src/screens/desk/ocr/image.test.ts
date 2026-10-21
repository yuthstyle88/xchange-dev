import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { prepareImage } from './image';
import { PassportReadError } from './passport-read';

describe('prepareImage', () => {
  const originalCreate = URL.createObjectURL;
  const originalRevoke = URL.revokeObjectURL;

  let imgWidth = 1200;
  let imgHeight = 800;
  let imgFail = false;
  let ctxStub: { translate: ReturnType<typeof vi.fn>; rotate: ReturnType<typeof vi.fn>; drawImage: ReturnType<typeof vi.fn> } | null;
  let createdCanvas: HTMLCanvasElement | undefined;
  let toBlobImpl: (cb: BlobCallback, type?: string, quality?: any) => void;

  beforeEach(() => {
    imgWidth = 1200;
    imgHeight = 800;
    imgFail = false;
    createdCanvas = undefined;
    ctxStub = {
      translate: vi.fn(),
      rotate: vi.fn(),
      drawImage: vi.fn(),
    };
    const defaultBlob = new Blob([new Uint8Array([1, 2, 3])], { type: 'image/jpeg' });
    toBlobImpl = (cb) => cb(defaultBlob);

    URL.createObjectURL = vi.fn(() => 'blob:test/1') as unknown as typeof URL.createObjectURL;
    URL.revokeObjectURL = vi.fn() as unknown as typeof URL.revokeObjectURL;

    class FakeImage {
      naturalWidth = imgWidth;
      naturalHeight = imgHeight;
      width = imgWidth;
      height = imgHeight;
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      set src(_v: string) {
        queueMicrotask(() => {
          if (imgFail) {
            this.onerror?.();
          } else {
            this.onload?.();
          }
        });
      }
    }
    vi.stubGlobal('Image', FakeImage);

    const origCreateElement = document.createElement.bind(document);
    vi.spyOn(document, 'createElement').mockImplementation(((tagName: string, options?: ElementCreationOptions) => {
      const el = origCreateElement(tagName, options);
      if (tagName.toLowerCase() === 'canvas') {
        const canvas = el as HTMLCanvasElement;
        createdCanvas = canvas;
        canvas.getContext = vi.fn(() => ctxStub as unknown as CanvasRenderingContext2D) as unknown as typeof canvas.getContext;
        canvas.toBlob = vi.fn((cb: BlobCallback, type?: string, quality?: any) => toBlobImpl(cb, type, quality));
      }
      return el;
    }) as typeof document.createElement);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
  });

  it('row 1: angle 0, <= 2600 px, <= 8 MB, jpeg returns the same Blob without canvas redraw', async () => {
    const file = new Blob([new Uint8Array([10, 20, 30])], { type: 'image/jpeg' });
    const out = await prepareImage(file, 0);
    expect(out).toBe(file);
    expect(createdCanvas).toBeUndefined();
  });

  it('row 2: 90 degrees swaps canvas width and height', async () => {
    imgWidth = 1200;
    imgHeight = 800;
    const file = new Blob([new Uint8Array([10, 20, 30])], { type: 'image/jpeg' });
    await prepareImage(file, 90);
    expect(createdCanvas).toBeDefined();
    expect(createdCanvas!.width).toBe(800);
    expect(createdCanvas!.height).toBe(1200);
  });

  it('row 3: a 0x0 image rejects with imageInvalid', async () => {
    imgWidth = 0;
    imgHeight = 0;
    const file = new Blob([new Uint8Array([10])], { type: 'image/jpeg' });
    await expect(prepareImage(file, 0)).rejects.toMatchObject({
      name: 'PassportReadError',
      kind: 'imageInvalid',
    });
  });

  it('row 4: no 2d context rejects with failed', async () => {
    ctxStub = null;
    const file = new Blob([new Uint8Array([10])], { type: 'image/jpeg' });
    await expect(prepareImage(file, 90)).rejects.toMatchObject({
      name: 'PassportReadError',
      kind: 'failed',
    });
  });

  it('row 5: when quality 0.88 is over 8 MB and 0.75 fits, returns the 0.75 blob', async () => {
    const bigBlob = { size: 8 * 1024 * 1024 + 1, type: 'image/jpeg' } as Blob;
    const fitBlob = new Blob([new Uint8Array([7, 5])], { type: 'image/jpeg' });
    const qualities: number[] = [];
    toBlobImpl = (cb, _type, quality) => {
      qualities.push(quality);
      cb(quality === 0.88 ? bigBlob : fitBlob);
    };

    const file = new Blob([new Uint8Array([10])], { type: 'image/jpeg' });
    const out = await prepareImage(file, 90);

    expect(out).toBe(fitBlob);
    expect(qualities).toEqual([0.88, 0.75]);
  });

  it('row 6: when the image fails to load, rejects with imageInvalid', async () => {
    imgFail = true;
    const file = new Blob([new Uint8Array([10])], { type: 'image/jpeg' });
    await expect(prepareImage(file, 0)).rejects.toThrow(PassportReadError);
    await expect(prepareImage(file, 0)).rejects.toMatchObject({
      kind: 'imageInvalid',
    });
  });
});
