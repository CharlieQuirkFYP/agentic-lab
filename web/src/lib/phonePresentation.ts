import type { PhoneActivity } from './phoneBenchmarkTypes.ts'

export const phoneDate = (value: number | null | undefined) => value == null ? 'Unknown' : new Date(value).toLocaleString()
export const metricValue = (value: number | string | boolean | null | undefined, unit = '') => value == null ? 'Unavailable' : typeof value === 'number' ? `${value.toLocaleString(undefined, { maximumFractionDigits: 2 })}${unit ? ` ${unit}` : ''}` : String(value)
export function configurationLabel(configuration: Record<string, unknown> | null, id: string | null) {
  const model = (purpose: string) => {
    const value = configuration?.[purpose]
    if (!value || typeof value !== 'object') return 'unknown'
    const fields = value as Record<string, unknown>
    return `${typeof fields.name === 'string' ? fields.name : 'unknown'}${typeof fields.sha256 === 'string' ? ` (${fields.sha256.slice(0, 12)})` : ''}`
  }
  return `${id ?? 'Unknown configuration'} · ASR ${model('asr')} · Reasoning ${model('reasoning')}`
}
export function activityLink(item: PhoneActivity, search: string) {
  const params = new URLSearchParams(search)
  params.set('installation_id', item.installation_id ?? '')
  params.set('return', search)
  if (item.kind === 'attempt' && item.context.conversation_id) params.set('detail_conversation_id', item.context.conversation_id)
  if (item.kind === 'legacy_run' && item.context.experiment_id) params.set('legacy_experiment_id', item.context.experiment_id)
  const kind = item.kind === 'conversation' ? 'conversations' : item.kind === 'sweep' ? 'sweeps' : 'runs'
  return `/phone/${kind}/${encodeURIComponent(item.entity_id)}?${params}`
}
