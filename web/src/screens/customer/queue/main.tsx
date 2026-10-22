// QW-2.0 (kiosk-shared-phone §5r F7, §5r.4a): /app/queue (web/pages/app-queue.html). A customer page: the audience
// is set before render, and the screen sits inside the customer shell, as /app/bookings.
import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { QueueRequestScreen } from './QueueRequest';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <CustomerShell route="/app/queue" tabletFlow>
        <QueueRequestScreen />
      </CustomerShell>
    ),
    root,
  );
}
