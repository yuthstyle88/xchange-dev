import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import type { DeskStore, PassportFields } from '../desk-store';
import { PassportOcr } from './PassportOcr';
import * as texts from './i18n';

vi.mock('./image', () => ({
  prepareImage: vi.fn(async (file: Blob) => file),
}));

const JPEG_FILE = new File([new Uint8Array([0xff, 0xd8, 0xff])], 'passport.jpg', {
  type: 'image/jpeg',
});

function createTestStore(overrides?: {
  passport?: File;
  fields?: Partial<PassportFields>;
  identityChecked?: boolean;
}) {
  const [passport, setPassport] = createSignal<File | undefined>(overrides?.passport);
  const [fields, setFields] = createSignal<PassportFields>({
    name: '',
    passportNo: '',
    nationality: '',
    birthDate: '',
    passportExpiry: '',
    ...overrides?.fields,
  });
  const [identityChecked, setIdentityChecked] = createSignal(overrides?.identityChecked ?? false);

  const postBytes = vi.fn();
  const api = { postBytes } as unknown as ApiClient;

  const setField = vi.fn((k: keyof PassportFields, v: string, _by: 'user' | 'ocr') => {
    setFields((prev) => ({ ...prev, [k]: v }));
  });
  const setIdentityCheckedMock = vi.fn((v: boolean) => {
    setIdentityChecked(v);
  });

  const store = {
    api,
    branch: () => 'XC-B01',
    passport,
    setPassport,
    fields,
    setField,
    identityChecked,
    setIdentityChecked: setIdentityCheckedMock,
  } as unknown as DeskStore;

  return {
    store,
    postBytes,
    setPassport,
    setFields,
    setField,
    setIdentityChecked: setIdentityCheckedMock,
  };
}

describe('PassportOcr', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.clearAllMocks();
  });

  function mount(store: DeskStore) {
    dispose = render(() => <PassportOcr store={store} />, host);
  }

  function getStatusText(): string {
    return host.querySelector('p[role="status"]')?.textContent?.trim() ?? '';
  }

  function getStopButton(): HTMLButtonElement | null {
    const buttons = Array.from(host.querySelectorAll<HTMLButtonElement>('.desk-ocr-actions button'));
    return buttons.find((b) => b.textContent?.trim() === texts.th.stop) ?? null;
  }

  // Row 1: a PDF file -> the pdfNotSupported text (mutation: the PDF check removed)
  it('row 1: shows pdfNotSupported when given a PDF file without server call', async () => {
    const { store, postBytes, setPassport } = createTestStore();
    mount(store);

    const pdfFile = new File([new Uint8Array([1, 2, 3])], 'passport.pdf', {
      type: 'application/pdf',
    });
    setPassport(pdfFile);

    await vi.waitFor(() => {
      expect(getStatusText()).toBe(texts.th.pdfNotSupported);
    });
    expect(postBytes).not.toHaveBeenCalled();
  });

  // Row 2: answer {fields: null} -> mrzFail text (mutation: the null branch removed)
  it('row 2: shows mrzFail text when server returns fields: null', async () => {
    const { store, postBytes, setPassport } = createTestStore();
    postBytes.mockResolvedValueOnce({ fields: null, score: 0 });
    mount(store);

    setPassport(JPEG_FILE);

    await vi.waitFor(() => {
      expect(getStatusText()).toBe(texts.th.mrzFail);
    });
  });

  // Row 3: 3 fields, score 5 -> filled with count 3, not partial, not kept (mutation: count off, or < 5 branch changed)
  it('row 3: shows filled with count 3, not partial, and not kept when score is 5', async () => {
    const { store, postBytes, setPassport } = createTestStore();
    postBytes.mockResolvedValueOnce({
      fields: {
        name: 'SOMCHAI',
        passportNo: 'AB1234567',
        nationality: 'THA',
      },
      score: 5,
    });
    mount(store);

    setPassport(JPEG_FILE);

    await vi.waitFor(() => {
      const status = getStatusText();
      expect(status).toContain(texts.th.filled.replace('{count}', '3'));
      expect(status).not.toContain(texts.th.partial);
      expect(status).not.toContain(texts.th.kept);
      expect(status).toContain(texts.th.notProof);
    });
    expect(store.fields().name).toBe('SOMCHAI');
    expect(store.fields().passportNo).toBe('AB1234567');
    expect(store.fields().nationality).toBe('THA');
  });

  // Row 4: score 3 -> the partial text (mutation: partial dropped)
  it('row 4: shows partial text when score is less than 5', async () => {
    const { store, postBytes, setPassport } = createTestStore();
    postBytes.mockResolvedValueOnce({
      fields: {
        name: 'SOMCHAI',
        passportNo: 'AB1234567',
        nationality: 'THA',
      },
      score: 3,
    });
    mount(store);

    setPassport(JPEG_FILE);

    await vi.waitFor(() => {
      const status = getStatusText();
      expect(status).toContain(texts.th.partial);
    });
  });

  // Row 5: the user edits name before answer -> name kept; the kept text (mutation: the snapshot comparison skipped)
  it('row 5: keeps user-edited name and shows kept text when user edits before answer', async () => {
    const { store, postBytes, setPassport } = createTestStore();
    let resolveServer: (val: any) => void;
    postBytes.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveServer = resolve;
      }),
    );
    mount(store);

    setPassport(JPEG_FILE);
    await vi.waitFor(() => expect(postBytes).toHaveBeenCalled());

    // User edits name while read is in flight
    store.setField('name', 'MANUAL_NAME', 'user');

    resolveServer!({
      fields: {
        name: 'OCR_NAME',
        passportNo: 'AB1234567',
      },
      score: 5,
    });

    await vi.waitFor(() => {
      const status = getStatusText();
      expect(status).toContain(texts.th.kept);
    });
    expect(store.fields().name).toBe('MANUAL_NAME');
    expect(store.fields().passportNo).toBe('AB1234567');
  });

  // Row 6: ApiError('passportReaderTimeout') -> timeout text / failed text (mutation: the error mapping collapsed)
  it('row 6: maps passportReaderTimeout to timeout text and other error to failed text', async () => {
    const { store, postBytes, setPassport } = createTestStore();
    postBytes.mockRejectedValueOnce(
      new ApiError({ code: 'passportReaderTimeout', status: 504, message: 'timeout' }),
    );
    mount(store);

    setPassport(JPEG_FILE);

    await vi.waitFor(() => {
      expect(getStatusText()).toBe(texts.th.timeout);
    });

    // A generic error maps to failed text (busy is left out: whether it gets its own text is undecided)
    postBytes.mockRejectedValueOnce(new Error('network down'));
    const file2 = new File([new Uint8Array([1])], 'passport2.jpg', { type: 'image/jpeg' });
    setPassport(file2);

    await vi.waitFor(() => {
      expect(getStatusText()).toBe(texts.th.failed);
    });
  });

  // Row 7: Stop while pending -> cancelled text; Stop button gone (mutation: abort or setReading skipped)
  it('row 7: cancels read and removes Stop button when Stop is clicked while pending', async () => {
    const { store, postBytes, setPassport } = createTestStore();
    postBytes.mockReturnValueOnce(new Promise(() => {}));
    mount(store);

    setPassport(JPEG_FILE);

    await vi.waitFor(() => {
      expect(getStopButton()).not.toBeNull();
    });

    getStopButton()!.click();

    await vi.waitFor(() => {
      expect(getStatusText()).toBe(texts.th.cancelled);
      expect(getStopButton()).toBeNull();
    });
  });

  // Row 8: the passport file is replaced -> unedited OCR fills cleared (mutation: the clear skipped)
  it('row 8: clears unedited OCR fills when passport file is replaced', async () => {
    const { store, postBytes, setPassport } = createTestStore();
    postBytes.mockResolvedValueOnce({
      fields: {
        name: 'JOHN',
        passportNo: 'A123',
      },
      score: 5,
    });
    mount(store);

    setPassport(JPEG_FILE);

    await vi.waitFor(() => {
      expect(store.fields().name).toBe('JOHN');
      expect(store.fields().passportNo).toBe('A123');
    });

    // Replace with placeholder file which doesn't trigger OCR
    setPassport(new File([], 'no-passport.png', { type: 'image/png' }));

    await vi.waitFor(() => {
      expect(store.fields().name).toBe('');
      expect(store.fields().passportNo).toBe('');
    });
  });

  // Row 9: a read starts -> setIdentityChecked(false) called (mutation: the call deleted)
  it('row 9: calls setIdentityChecked(false) when a read starts', async () => {
    const { store, postBytes, setPassport, setIdentityChecked } = createTestStore({
      identityChecked: true,
    });
    postBytes.mockReturnValueOnce(new Promise(() => {}));
    mount(store);

    setPassport(JPEG_FILE);

    await vi.waitFor(() => {
      expect(setIdentityChecked).toHaveBeenCalledWith(false);
    });
  });

  // Row 10: file named no-passport.png -> no call; empty status (mutation: the placeholder check removed)
  it('row 10: does not call server and leaves status empty for no-passport.png', async () => {
    const { store, postBytes, setPassport } = createTestStore();
    mount(store);

    setPassport(new File([], 'no-passport.png', { type: 'image/png' }));

    await new Promise((r) => setTimeout(r, 20));
    expect(postBytes).not.toHaveBeenCalled();
    expect(getStatusText()).toBe('');
  });
});
