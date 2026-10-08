import type { JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import { AppShell } from '../../shared/ui/AppShell';
import * as guideI18n from './i18n';
import './guide.css';

const t = screenT('staff-guide', guideI18n);

export interface StaffGuideProps {
  print?: () => void;
}

export function StaffGuide(props: StaffGuideProps): JSX.Element {
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
        <nav><a href="/">{t('back')}</a> · <button type="button" onClick={handlePrint}>{t('print')}</button></nav>
      }
    >
      <section>
        <h2>{t('section1Heading')}</h2>
        <p>{t('section1Body')}</p>
      </section>
      <section>
        <h2>{t('section2Heading')}</h2>
        <p>{t('section2Body')}</p>
      </section>
      <section>
        <h2>{t('section3Heading')}</h2>
        <p>{t('section3Body')}</p>
      </section>
      <section>
        <h2>{t('section4Heading')}</h2>
        <p>{t('section4Body')}</p>
      </section>
      <section>
        <h2>{t('section5Heading')}</h2>
        <p>{t('section5Body1')}</p>
        <p>{t('section5Body2')}</p>
      </section>
      <section>
        <h2>{t('section6Heading')}</h2>
        <p>{t('section6Body')}</p>
      </section>
    </AppShell>
  );
}
