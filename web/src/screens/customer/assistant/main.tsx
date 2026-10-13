import { render } from 'solid-js/web';
import { setAudience } from '../../../shared/i18n';
import { CustomerShell } from '../_shell/Shell';
import { AssistantScreen } from './AssistantScreen';
import './assistant.css';

setAudience('customer');

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <CustomerShell route="/app/assistant">
        <AssistantScreen search={window.location.search} />
      </CustomerShell>
    ),
    root,
  );
}
