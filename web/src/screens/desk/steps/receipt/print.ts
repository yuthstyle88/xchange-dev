import type { ApiClient } from '../../../../shared/api/client';
import type { ReceiptDocumentViewData } from '../../../../shared/api/generated/receipt_document';
import { screenT } from '../../../../shared/i18n';
import { wordedError } from '../../../../shared/i18n/worded-error';
import { ReceiptFrame } from '../../../../shared/receipt/ReceiptFrame';
import * as texts from './i18n';

const t = screenT('desk.steps.receipt', texts);

export async function printSaved(
  api: ApiClient,
  documentNumber: string,
): Promise<'printed' | { failed: string }> {
  try {
    const res = await api.view<ReceiptDocumentViewData>('receipt_document', {
      filter: { number: documentNumber },
    });
    const html = res.data?.receipt?.html;
    if (!html) {
      return { failed: t('receiptMissing') };
    }
    await ReceiptFrame.print(html);
    return 'printed';
  } catch (err) {
    return { failed: wordedError(err) };
  }
}
