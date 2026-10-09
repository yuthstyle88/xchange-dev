import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { AppShell } from '../../shared/ui/AppShell';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';
import * as screenI18n from './i18n';
import { screenT } from '../../shared/i18n';

const root = document.getElementById('app') ?? document.body;

const url = new URL(window.location.href);
const branch = url.searchParams.get('branch') || 'MAIN';
const api = new ApiClient();
const t = screenT('kiosk-currencies', screenI18n);

render(
  () => (
    <AppShell title={t('heading')} look="kiosk">
      <KioskCurrenciesScreen api={api} branch={branch} />
    </AppShell>
  ),
  root,
);
