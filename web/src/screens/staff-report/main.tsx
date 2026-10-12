// V3-staff-report #1350: the entry of /staff/report (design `xchange-v3-wave3-staff-report.md` B.3, D-B7, D-B8). It
// renders the report inside the shared AppShell (LK-1); print.css hides the shell's chrome on paper.
import { render } from 'solid-js/web';
import { screenT } from '../../shared/i18n';
import { AppShell } from '../../shared/ui/AppShell';
import { en, th } from './i18n';
import { StaffReport } from './StaffReport';
import './print.css';
import './staff-report.css';

const root = document.getElementById('app') ?? document.body;
const t = screenT('staff-report', { th, en });

render(
  () => (
    <AppShell title={t('title')}>
      <StaffReport />
    </AppShell>
  ),
  root,
);
