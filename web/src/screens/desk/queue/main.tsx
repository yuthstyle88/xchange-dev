// B06 `/desk/queue` (design docs/design/blueprint/queue-tickets.md §4.4, §5; FE-desk skeleton, #564): the teller's queue
// screen inside the shared shell.
import { render } from 'solid-js/web';
import { screenT } from '../../../shared/i18n';
import { AppShell } from '../../../shared/ui/AppShell';
import { QueueScreen } from './QueueScreen';
import { en, th } from './i18n';

const root = document.getElementById('app') ?? document.body;
const t = screenT('desk.queue', { th, en });

render(
  () => (
    <AppShell title={t('title')}>
      <QueueScreen />
    </AppShell>
  ),
  root,
);
