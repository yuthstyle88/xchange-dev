import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { JourneyScreen } from './JourneyScreen';
import './journey.css';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <CustomerShell route="/app/journey">
        <JourneyScreen />
      </CustomerShell>
    ),
    root,
  );
}
