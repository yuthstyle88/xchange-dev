// XC Counter CA-5 (docs/design/xc-counter-app.md §3, §4): "สแกนพาสปอร์ต" beside the passport file field, only inside
// the app and only when the owner set `scanner` on for this counter. Skeleton by claude/bbe9ce8b; the body is CA-5's.
//
// A scan is handed in exactly as an attached file: after the same `fileRefusal(file)` check `onFile` uses (exported by
// ./step), `store.setPassport(new File([blob], 'passport-scan.jpg', { type: 'image/jpeg' }))`. The existing upload and
// OCR run from there. Texts: the `scan*` keys in ./i18n.ts.
import { createSignal, onCleanup, Show, type JSX } from 'solid-js';
import { screenT } from '../../../../shared/i18n';
import { getXcCounter, type ScanRefusal } from '../../../../shared/xc-counter/bridge';
import type { DeskStore } from '../../desk-store';
import * as texts from './i18n';
import { fileRefusal } from './step';

const t = screenT('desk.steps.customer', texts);

const REFUSAL_KEYS: Record<ScanRefusal, keyof typeof texts.th> = {
  noScanner: 'scanNoScanner',
  busy: 'scanBusy',
  timeout: 'scanTimeout',
  coverOpen: 'scanCoverOpen',
  device: 'scanDevice',
};

export function ScanPassportButton(props: { store: DeskStore }): JSX.Element {
  const counter = getXcCounter();
  if (!counter) return null;

  const [enabled, setEnabled] = createSignal(false);
  const [scanning, setScanning] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>();
  let alive = true;
  onCleanup(() => {
    alive = false;
  });

  void counter.info().then(
    (info) => {
      if (alive && info.capabilities.scanner) setEnabled(true);
    },
    () => {},
  );

  async function onScan(): Promise<void> {
    if (!counter || scanning()) return;
    setScanning(true);
    setError(undefined);
    try {
      const res = await counter.scanPassport();
      if (!alive) return;
      if (!res.ok) {
        setError(t(REFUSAL_KEYS[res.reason] ?? 'scanDevice'));
        return;
      }
      const file = new File([res.file], 'passport-scan.jpg', { type: 'image/jpeg' });
      const refusal = fileRefusal(file);
      if (refusal) {
        setError(refusal);
        return;
      }
      props.store.setPassport(file);
    } catch {
      if (alive) setError(t('scanDevice'));
    } finally {
      if (alive) setScanning(false);
    }
  }

  return (
    <Show when={enabled()}>
      <div class="desk-scan-passport">
        <button type="button" disabled={scanning()} onClick={() => void onScan()}>
          {scanning() ? t('scanning') : t('scanButton')}
        </button>
        <Show when={error()}>
          <p class="desk-error" role="alert">
            {error()}
          </p>
        </Show>
      </div>
    </Show>
  );
}
