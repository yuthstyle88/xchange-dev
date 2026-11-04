import { render } from 'solid-js/web';
import { screenT } from '../../shared/i18n';
import { AppShell } from '../../shared/ui/AppShell';
import { BackdatedScreen } from './BackdatedScreen';
import { en, th } from './i18n';

const root = document.getElementById('app') ?? document.body;
const t = screenT('desk-backdated', { th, en });

render(
  () => (
    <AppShell title={t('title')}>
      <BackdatedScreen />
    </AppShell>
  ),
  root,
);
