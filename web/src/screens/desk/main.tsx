import { render } from 'solid-js/web';
import { screenT } from '../../shared/i18n';
import { AppShell } from '../../shared/ui/AppShell';
import { DeskPage } from './DeskPage';
import { createDeskStore, type DeskStore } from './desk-store';
import { openEntry } from './entries';
import * as screenI18n from './i18n';
import { collectSteps, type DeskStep } from './steps';
import { Summary } from './Summary';

const root = document.getElementById('app') ?? document.body;
const t = screenT('desk', screenI18n);

const stepModules = import.meta.glob('./steps/*/step.tsx', { eager: true });
const steps: DeskStep<DeskStore>[] = collectSteps(stepModules);
const store = createDeskStore();
void openEntry(store);

render(
  () => (
    <AppShell title={t('title')}>
      <DeskPage store={store} steps={steps} aside={<Summary store={store} />} />
    </AppShell>
  ),
  root,
);
