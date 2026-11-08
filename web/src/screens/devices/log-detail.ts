import { formatBangkokDateTime } from '../../shared/format/bangkok-time';
import { detailKeyLabel, detailValueLabel } from './labels';

export interface DetailLine {
  label: string;
  text: string;
  full?: string;
}

const UUID_RE = /^([a-z][a-z0-9_-]*:)?([0-9a-f]{8})-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function shortId(value: string): string {
  const m = UUID_RE.exec(value);
  return m ? `${m[1] ?? ''}${m[2]!}` : value;
}

export function detailLines(detail: string): DetailLine[] {
  if (!detail) return [{ label: '', text: '—' }];
  let parsed: unknown;
  try {
    parsed = JSON.parse(detail);
  } catch {
    return [{ label: '', text: detail }];
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return [{ label: '', text: detail }];
  }
  const entries = Object.entries(parsed as Record<string, unknown>);
  if (entries.length === 0) return [{ label: '', text: '—' }];
  return entries.map(([key, v]) => {
    const label = detailKeyLabel(key);
    if (v === null) return { label, text: '—' };
    if (typeof v === 'boolean') return { label, text: detailValueLabel(String(v)) };
    if (typeof v === 'number') return { label, text: String(v) };
    if (typeof v === 'string') {
      if (key === 'queuedAt' || key === 'leasedAt' || key === 'reportedAt') {
        return { label, text: formatBangkokDateTime(v) };
      }
      if (key === 'state' || key === 'source' || key === 'mode') {
        return { label, text: detailValueLabel(v) };
      }
      const text = shortId(v);
      return text !== v ? { label, text, full: v } : { label, text };
    }
    return { label, text: JSON.stringify(v) };
  });
}
