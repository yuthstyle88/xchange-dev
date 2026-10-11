import { Show } from 'solid-js';
import { render } from 'solid-js/web';
import { screenT } from '../../shared/i18n';
import { sharedSession } from '../../shared/session/session';
import { pageBranch } from '../../shared/session/page-branch';
import { AppShell } from '../../shared/ui/AppShell';
import * as screenI18n from './i18n';
import { KioskQueueScreen } from './KioskQueueScreen';

const root = document.getElementById('app') ?? document.body;
const t = screenT('kiosk-queue', screenI18n);
const branch = pageBranch(sharedSession());

render(
  () => (
    <AppShell title={t('title')}>
      <Show when={branch()} keyed>
        {(b) => <KioskQueueScreen branch={b} />}
      </Show>
    </AppShell>
  ),
  root,
);
