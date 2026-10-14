// VF-10 (#1401) customer verification screen C10 (`/app/verification`).
// A customer page: audience is set before render (languages.md §3.1), and the screen sits inside CustomerShell.
import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { VerificationScreen } from './VerificationScreen';

setAudience('customer');

export function mount(
  container: HTMLElement = document.getElementById('root') ?? document.body,
) {
  return render(
    () => (
      <CustomerShell route="/app">
        <VerificationScreen />
      </CustomerShell>
    ),
    container,
  );
}

const root = document.getElementById('root');
if (root) {
  mount(root);
}
