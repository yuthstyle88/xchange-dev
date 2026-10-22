// APP-7 (#998) the C14 sign-in page, `/app/sign-in` (web/pages/app-sign-in.html). A customer page: the audience is
// set before render (languages.md §3.1), and the screen sits inside the customer shell. The contract has no sign-in
// route, so the shell is given `/app`, the app's home.
import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { SignIn } from './SignIn';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <CustomerShell route="/app" tabletFlow>
        <SignIn navigate={(url) => window.location.assign(url)} />
      </CustomerShell>
    ),
    root,
  );
}
