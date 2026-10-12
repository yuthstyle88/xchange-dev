// #1349 V3-documents (design §A.3): the entry of `/staff/documents`. The query's first `ref` counts (#139).
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { screenT } from '../../shared/i18n';
import { AppShell } from '../../shared/ui/AppShell';
import { STAFF } from './audiences';
import { DocumentsScreen } from './DocumentsScreen';
import * as table from './i18n';

const root = document.getElementById('app') ?? document.body;
const t = screenT('documents', table);
const query = new URLSearchParams(location.search);

render(
  () => (
    <AppShell title={t('title')}>
      <DocumentsScreen
        api={new ApiClient()}
        audience={STAFF}
        reference={query.get('ref') ?? ''}
        branch={''}
        replace={(url) => location.replace(url)}
      />
    </AppShell>
  ),
  root,
);
