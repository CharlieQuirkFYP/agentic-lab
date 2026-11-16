import type { StoredPhoneObservation, RecordReference } from '../../../shared/phone-benchmarks/types.ts'
import { phoneQuery, type PhoneFilters, type PhoneOverview, type PhoneObservationPage, type LegacyPhonePage, type LegacyPhoneObservation } from './phoneBenchmarkTypes.ts'

const API_BASE = import.meta.env?.VITE_API_BASE_URL ?? '/api/v1'
async function request<T>(path: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(`${API_BASE}/phone/${path}`, { signal })
  if (!response.ok) { const body = await response.json().catch(() => ({})); throw new Error(body.error ?? `Phone metrics request failed (${response.status})`) }
  return response.json() as Promise<T>
}
export const getPhoneOverview = (filters: PhoneFilters, signal?: AbortSignal, asOf?: string) => request<PhoneOverview>(`overview?${phoneQuery(filters, { as_of: asOf })}`, signal)

/** Every page and legacy observation belongs to the same finite server watermark. */
export async function getPhoneSnapshot(filters: PhoneFilters, asOf: string, signal?: AbortSignal) {
  const records: StoredPhoneObservation[] = []
  const legacy: LegacyPhoneObservation[] = []
  const selected_record_keys: RecordReference[] = []
  let cursor: string | undefined
  const seen = new Set<string>()
  do {
    const page = await request<PhoneObservationPage>(`observations?${phoneQuery(filters, { as_of: asOf, cursor, limit: 100 })}`, signal)
    if (page.as_of !== asOf) throw new Error('Observation snapshot changed while loading. Refresh and try again.')
    records.push(...page.records)
    selected_record_keys.push(...page.selected_record_keys ?? page.records.map(item => ({ installation_id: item.device.installation_id, record_id: item.record.record_id })))
    cursor = page.next_cursor ?? undefined
    if (!cursor && records.length !== page.total) throw new Error('Observation snapshot is incomplete. Refresh and try again.')
    if (cursor && seen.has(cursor)) throw new Error('Observation pagination did not advance.')
    if (cursor) seen.add(cursor)
  } while (cursor)
  seen.clear()
  do {
    const page = await request<LegacyPhonePage>(`legacy-observations?${phoneQuery(filters, { as_of: asOf, cursor, limit: 100 })}`, signal)
    if (page.as_of !== asOf) throw new Error('Legacy snapshot changed while loading. Refresh and try again.')
    legacy.push(...page.batches)
    cursor = page.next_cursor ?? undefined
    if (!cursor && legacy.length !== page.total) throw new Error('Legacy snapshot is incomplete. Refresh and try again.')
    if (cursor && seen.has(cursor)) throw new Error('Legacy pagination did not advance.')
    if (cursor) seen.add(cursor)
  } while (cursor)
  return { records, legacy, as_of: asOf, selected_record_keys, filtered: Object.values(filters).some(value => value !== undefined) }
}
