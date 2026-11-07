import type { JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import { sharedSession } from '../../shared/session/session';
import { AppShell } from '../../shared/ui/AppShell';
import * as guideI18n from './i18n';
import '../../shared/ui/card.css';
import './guide.css';

const t = screenT('staff-guide', guideI18n);
// / has no permission check of its own, so follow the shift-close certification permission.
const ADMIN_BACK_PERMISSION = 'ShiftCloseCertify';

export interface StaffGuideProps {
  print?: () => void;
}

export function StaffGuide(props: StaffGuideProps): JSX.Element {
  const toAdmin = () => sharedSession().me()?.permissions.includes(ADMIN_BACK_PERMISSION) ?? false;
  const handlePrint = () => {
    if (props.print) {
      props.print();
    } else {
      window.print();
    }
  };

  return (
    <AppShell
      title={t('title')}
      nav={
        <nav><a href={toAdmin() ? '/' : '/staff'}>{toAdmin() ? t('back') : t('backStaff')}</a> · <button type="button" onClick={handlePrint}>{t('print')}</button></nav>
      }
    >
      <div class="staff-guide">
        <section class="card">
          <h2>{t('section1Heading')}</h2>
          <p>{t('section1Body')}</p>
        </section>
        <section class="card">
          <h2>{t('section2Heading')}</h2>
          <p>{t('section2Body')}</p>
        </section>
        <section class="card">
          <h2>{t('section3Heading')}</h2>
          <p>{t('section3Body')}</p>
        </section>
        <section class="card">
          <h2>{t('section4Heading')}</h2>
          <p>{t('section4Body')}</p>
        </section>
        <section class="card">
          <h2>{t('section5Heading')}</h2>
          <p>{t('section5Body1')}</p>
          <p>{t('section5Body2')}</p>
        </section>
        <section class="card">
          <h2>{t('section6Heading')}</h2>
          <p>{t('section6Body')}</p>
        </section>
      </div>
    </AppShell>
  );
}
