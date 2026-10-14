// BK-8 #1409 (B02 `/desk/bookings`, design `docs/design/blueprint/booking-slots.md` §4.4).
// STUB by the up-front test writer: the entry renders the shared shell around the screen.
// The implementer keeps the shell and fills the screen.
import { render } from 'solid-js/web';
import { screenT } from '../../../shared/i18n';
import { AppShell } from '../../../shared/ui/AppShell';
import { DeskBookingsScreen } from './DeskBookingsScreen';
import { en, th } from './i18n';

const root = document.getElementById('app') ?? document.body;
const t = screenT('desk.bookings', { th, en });

render(
  () => (
    <AppShell title={t('title')}>
      <DeskBookingsScreen />
    </AppShell>
  ),
  root,
);
