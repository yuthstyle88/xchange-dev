// TD-3 #1291 (B04 `/desk/transactions/{id}`, design `docs/design/blueprint/desk-transaction-detail.md`).
// STUB by the up-front test writer: the entry renders the shared shell around the screen. The implementer keeps the
// shell and fills the screen.
import { render } from 'solid-js/web';
import { screenT } from '../../shared/i18n';
import { AppShell } from '../../shared/ui/AppShell';
import { en, th } from './i18n';
import { TransactionScreen } from './TransactionScreen';

const root = document.getElementById('app') ?? document.body;
const t = screenT('desk-transaction', { th, en });

render(
  () => (
    <AppShell title={t('title')}>
      <TransactionScreen pathname={window.location.pathname} search={window.location.search} />
    </AppShell>
  ),
  root,
);
