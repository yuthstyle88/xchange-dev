import type { JSX } from 'solid-js';
import { Show } from 'solid-js';
import { screenT } from '../../shared/i18n';
import { Card } from '../../shared/ui/Card';
import { en, th } from './i18n';
import './devices-receipt-preview.css';

const t = screenT('devices-receipt-preview', { th, en });

export function DevicesReceiptPreview(props: { search: string }): JSX.Element {
  let frameEl: HTMLIFrameElement | undefined;
  const q = () => new URLSearchParams(props.search);
  const branch = () => q().get('branch') ?? '';
  const ref = () => q().get('ref') ?? '';
  const backHref = () => (branch() ? `/devices?branch=${encodeURIComponent(branch())}` : '/devices');
  const slipUrl = () => {
    const p = new URLSearchParams();
    if (branch()) p.set('branch', branch());
    p.set('ref', ref());
    return `/devices/receipt?${p.toString().replace(/\+/g, '%20')}`;
  };

  return (
    <div class="xc-receipt-preview">
      <div class="xc-receipt-preview-bar">
        <a class="xc-receipt-preview-back" data-action="back" href={backHref()}>
          {t('back')}
        </a>
        <Show when={ref()}>
          <button
            type="button"
            class="xc-btn xc-btn--primary"
            data-action="print"
            onClick={() => frameEl?.contentWindow?.print()}
          >
            {t('print')}
          </button>
        </Show>
      </div>
      <Card class="xc-receipt-preview-card">
        <Show when={ref()} fallback={<p data-notice="no-ref">{t('noRef')}</p>}>
          <iframe ref={frameEl} src={slipUrl()} title={ref()} />
        </Show>
      </Card>
    </div>
  );
}
