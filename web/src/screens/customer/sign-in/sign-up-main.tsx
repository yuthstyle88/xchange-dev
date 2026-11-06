// APP-7 (#998, #6314) the C14 sign-up page, `/app/sign-up` (web/pages/app-sign-up.html). A customer page: the audience is
// set before render (languages.md §3.1), and the screen sits inside the customer shell with `/app/sign-up`.
import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { SignUp } from './SignUp';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <CustomerShell route="/app/sign-up" tabletFlow>
        <SignUp navigate={(url) => window.location.assign(url)} />
      </CustomerShell>
    ),
    root,
  );
}
