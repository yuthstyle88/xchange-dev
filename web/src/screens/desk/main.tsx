import { render } from 'solid-js/web';
import { screenT } from '../../shared/i18n';
import { AppShell } from '../../shared/ui/AppShell';
import { DeskPage } from './DeskPage';
import { createFrameStore } from './frame-store';
import * as screenI18n from './i18n';
import { collectSteps } from './steps';

const root = document.getElementById('app') ?? document.body;
const t = screenT('desk', screenI18n);

const stepModules = import.meta.glob('./steps/*/step.tsx', { eager: true });
const steps = collectSteps(stepModules);
const store = createFrameStore(steps.map((s) => s.index));

render(
  () => (
    <AppShell title={t('title')}>
      <DeskPage store={store} steps={steps} />
    </AppShell>
  ),
  root,
);
