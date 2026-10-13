import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { HistoryScreen } from './HistoryScreen';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <CustomerShell route="/app/history">
        <HistoryScreen />
      </CustomerShell>
    ),
    root,
  );
}
