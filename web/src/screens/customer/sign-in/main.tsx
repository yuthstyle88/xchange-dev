// APP-7 (#998, #6314) the C14 sign-in page, `/app/sign-in` (web/pages/app-sign-in.html). A customer page: the audience is
// set before render (languages.md §3.1), and the screen sits inside the customer shell with `/app/sign-in`.
import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { SignIn } from './SignIn';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <CustomerShell route="/app/sign-in" tabletFlow>
        <SignIn navigate={(url) => window.location.assign(url)} />
      </CustomerShell>
    ),
    root,
  );
}
