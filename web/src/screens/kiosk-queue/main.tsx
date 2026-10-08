import { render } from 'solid-js/web';
import { screenT } from '../../shared/i18n';
import { setQueueBranch } from '../../shared/queue/queue-store';
import { AppShell } from '../../shared/ui/AppShell';
import * as screenI18n from './i18n';
import { KioskQueueScreen } from './KioskQueueScreen';

const root = document.getElementById('app') ?? document.body;

const url = new URL(window.location.href);
const branch = url.searchParams.get('branch') || 'MAIN';
setQueueBranch(branch);

const t = screenT('kiosk-queue', screenI18n);

render(
  () => (
    <AppShell title={t('title')}>
      <KioskQueueScreen branch={branch} />
    </AppShell>
  ),
  root,
);
