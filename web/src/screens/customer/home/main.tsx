// APP-8 (C01 customer home screen, `/app`).
import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { HomeScreen } from './HomeScreen';
import './home.css';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <CustomerShell route="/app">
        <HomeScreen search={window.location.search} />
      </CustomerShell>
    ),
    root,
  );
}
