import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { screenT } from '../../shared/i18n';
import { sharedSession } from '../../shared/session/session';
import { pageBranch } from '../../shared/session/page-branch';
import { AppShell } from '../../shared/ui/AppShell';
import * as screenI18n from './i18n';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

const root = document.getElementById('app') ?? document.body;
const branch = pageBranch(sharedSession());
const api = new ApiClient();
const t = screenT('kiosk-currencies', screenI18n);

render(
  () => (
    <AppShell title={t('heading')} look="kiosk">
      <KioskCurrenciesScreen api={api} branch={branch()} />
    </AppShell>
  ),
  root,
);
