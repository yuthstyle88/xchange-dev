import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { RateAlertsScreen } from './RateAlerts';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <CustomerShell route="/app/rate-alerts">
        <RateAlertsScreen />
      </CustomerShell>
    ),
    root,
  );
}
