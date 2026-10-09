import type { JSX } from 'solid-js';
import { screenT } from '../../shared/i18n';
import { QueuePanel } from '../../shared/ui/QueuePanel';
import * as screenI18n from './i18n';

export function KioskQueueScreen(props: { branch: string }): JSX.Element {
  const t = screenT('kiosk-queue', screenI18n);

  return (
    <div class="xc-content queue-page">
      <nav>
        <a href="/">{t('back')}</a>
      </nav>
      <h2>{t('heading')}</h2>
      <p>{t('intro')}</p>
      <QueuePanel branch={props.branch} shown={true} />
    </div>
  );
}
