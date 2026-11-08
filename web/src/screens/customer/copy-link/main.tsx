import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { CopyLink } from './CopyLink';
import './copy-link.css';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <CustomerShell route="/c">
        <CopyLink fetch={(url, init) => fetch(url, init)} />
      </CustomerShell>
    ),
    root,
  );
}
