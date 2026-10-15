import { render } from 'solid-js/web';
import { screenT } from '../../../shared/i18n';
import { AppShell } from '../../../shared/ui/AppShell';
import { en, th } from './i18n';
import { VerificationScreen } from './VerificationScreen';

export { VerificationScreen };

export function mount(
  container: HTMLElement = document.getElementById('app') ?? document.getElementById('root') ?? document.body,
) {
  const st = screenT('operations.verification', { th, en });
  return render(
    () => (
      <AppShell title={st('title')}>
        <VerificationScreen />
      </AppShell>
    ),
    container,
  );
}

const root = document.getElementById('app') ?? document.getElementById('root');
if (root) {
  mount(root);
}
