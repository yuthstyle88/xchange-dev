import { render } from 'solid-js/web';
import { screenT } from '../../shared/i18n';
import { AppShell } from '../../shared/ui/AppShell';
import { DevicesReceiptPreview } from './DevicesReceiptPreview';
import { en, th } from './i18n';

const t = screenT('devices-receipt-preview', { th, en });

render(
  () => (
    <AppShell title={t('heading')}>
      <DevicesReceiptPreview search={window.location.search} />
    </AppShell>
  ),
  document.getElementById('app') ?? document.body,
);
