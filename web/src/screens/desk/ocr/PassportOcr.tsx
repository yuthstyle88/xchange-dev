// Step 1 passport OCR controls, messages and snapshot fill rule (design xchange-v3-wave3-desk.md §6; #542).
import { createEffect, createSignal, onCleanup, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import type { DeskStore, PassportFields } from '../desk-store';
import * as texts from './i18n';
import { prepareImage, type Angle } from './image';
import { PassportReadError, readPassportOnServer } from './passport-read';
import './ocr.css';

const t = screenT('desk.ocr', texts);

const FIELD_KEYS: readonly (keyof PassportFields)[] = [
  'name',
  'passportNo',
  'nationality',
  'birthDate',
  'passportExpiry',
];

type OcrStatus =
  | { kind: 'preparing' }
  | { kind: 'mrzFail' }
  | { kind: 'cancelled' }
  | { kind: 'timeout' }
  | { kind: 'failed' }
  | { kind: 'pdfNotSupported' }
  | { kind: 'filled'; count: number; partial: boolean; kept: boolean };

function isPdf(file: File): boolean {
  return file.type === 'application/pdf' || /\.pdf$/i.test(file.name);
}

export function PassportOcr(props: { store: DeskStore }): JSX.Element {
  const store = props.store;
  const [angle, setAngle] = createSignal<Angle>(0);
  const [reading, setReading] = createSignal(false);
  const [status, setStatus] = createSignal<OcrStatus | undefined>();

  let activeCtl: AbortController | undefined;
  let filledByOcr: Partial<Record<keyof PassportFields, string>> = {};
  const initialFile = store.passport();
  let lastFile: File | undefined =
    initialFile && (store.fields().name !== '' || store.identityChecked()) ? initialFile : undefined;

  function abortCurrent(): void {
    if (activeCtl) {
      activeCtl.abort();
      activeCtl = undefined;
    }
    setReading(false);
  }

  function clearUnchangedOcrFills(): void {
    const current = store.fields();
    for (const k of FIELD_KEYS) {
      const filled = filledByOcr[k];
      if (filled !== undefined && current[k] === filled) {
        store.setField(k, '', 'ocr');
      }
    }
    filledByOcr = {};
  }

  onCleanup(() => {
    abortCurrent();
  });

  async function runRead(file: File): Promise<void> {
    abortCurrent();
    if (isPdf(file)) {
      setStatus({ kind: 'pdfNotSupported' });
      return;
    }

    const ctl = new AbortController();
    activeCtl = ctl;
    const snapshot: PassportFields = { ...store.fields() };
    store.setIdentityChecked(false);
    setReading(true);
    setStatus({ kind: 'preparing' });

    try {
      const prepared = await prepareImage(file, angle());
      if (ctl.signal.aborted || activeCtl !== ctl) return;
      const answer = await readPassportOnServer(store.api, prepared, {
        branch: store.branch(),
        signal: ctl.signal,
      });
      if (ctl.signal.aborted || activeCtl !== ctl) return;

      if (!answer.fields) {
        setStatus({ kind: 'mrzFail' });
        return;
      }

      let count = 0;
      let kept = false;
      const nextFills: Partial<Record<keyof PassportFields, string>> = {};
      for (const k of FIELD_KEYS) {
        const val = answer.fields[k];
        if (typeof val === 'string' && val !== '') {
          if (store.fields()[k] === snapshot[k]) {
            store.setField(k, val, 'ocr');
            nextFills[k] = val;
            count += 1;
          } else {
            kept = true;
          }
        }
      }
      filledByOcr = nextFills;

      if (count === 0 && !kept) {
        setStatus({ kind: 'mrzFail' });
        return;
      }

      setStatus({
        kind: 'filled',
        count,
        partial: answer.score < 5,
        kept,
      });
    } catch (err) {
      if (activeCtl !== ctl) return;
      if (ctl.signal.aborted || (err instanceof PassportReadError && err.kind === 'aborted')) {
        setStatus({ kind: 'cancelled' });
        return;
      }
      if (err instanceof PassportReadError && err.kind === 'timeout') {
        setStatus({ kind: 'timeout' });
        return;
      }
      // shortcut: O-1 open error kinds (busy, notConfigured, imageTooLarge, imageInvalid, notAllowed) use failed text until ruled
      setStatus({ kind: 'failed' });
    } finally {
      if (activeCtl === ctl) {
        activeCtl = undefined;
        setReading(false);
      }
    }
  }

  function stopRead(): void {
    abortCurrent();
    setStatus({ kind: 'cancelled' });
  }

  createEffect(() => {
    const file = store.passport();
    if (file === lastFile) return;

    if (lastFile !== undefined || activeCtl !== undefined) {
      abortCurrent();
      clearUnchangedOcrFills();
    }
    lastFile = file;

    if (!file || file.name === 'no-passport.png') {
      setStatus(undefined);
      return;
    }
    void runRead(file);
  });

  function statusText(): string {
    const s = status();
    if (!s) return '';
    switch (s.kind) {
      case 'preparing':
        return t('preparing');
      case 'mrzFail':
        return t('mrzFail');
      case 'cancelled':
        return t('cancelled');
      case 'timeout':
        return t('timeout');
      case 'failed':
        return t('failed');
      case 'pdfNotSupported':
        return t('pdfNotSupported');
      case 'filled': {
        const parts = [t('filled', { count: s.count })];
        if (s.partial) parts.push(t('partial'));
        if (s.kept) parts.push(t('kept'));
        parts.push(t('notProof'));
        return parts.join(' · ');
      }
    }
  }

  return (
    <div class="desk-ocr">
      <div class="desk-ocr-controls">
        <label class="desk-field">
          <span>{t('rotateLabel')}</span>
          <select
            name="ocrAngle"
            value={String(angle())}
            onChange={(e) => setAngle(Number(e.currentTarget.value) as Angle)}
          >
            <option value="0">{t('rotate0')}</option>
            <option value="90">{t('rotate90')}</option>
            <option value="180">{t('rotate180')}</option>
            <option value="270">{t('rotate270')}</option>
          </select>
        </label>
        <div class="desk-ocr-actions">
          <button
            type="button"
            disabled={reading() || !store.passport()}
            onClick={() => {
              const file = store.passport();
              if (file) void runRead(file);
            }}
          >
            {t('read')}
          </button>
          <Show when={reading()}>
            <button type="button" onClick={stopRead}>
              {t('stop')}
            </button>
          </Show>
        </div>
      </div>
      <p class="desk-ocr-status" role="status" aria-live="polite">
        {statusText()}
      </p>
    </div>
  );
}
