import { useState } from 'react'
import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { getPhoneSnapshot } from '@/lib/phoneBenchmarkApi'
import { legacyActivities, projectedActivity } from '@/lib/phoneProjection'
import type { PhoneActivity, PhoneFilters, PhoneOverview } from '@/lib/phoneBenchmarkTypes'
import { projectLegacyObservations, projectObservations } from '../../../../shared/phone-benchmarks/index.ts'
import { ModelConfiguration, Status } from './PhoneMetrics'
import { activityLink, phoneDate } from '@/lib/phonePresentation'

export default function PhoneHistory({ overview, filters, search, view, setView }: { overview: PhoneOverview; filters: PhoneFilters; search: string; view: string; setView: (view: string) => void }) {
  const [all, setAll] = useState<{ key: string; asOf: string; conversations: PhoneActivity[]; sweeps: PhoneActivity[]; runs: PhoneActivity[] } | null>(null)
  const [pageSelection, setPageSelection] = useState<{ key: string; page: number }>({ key: '', page: 0 })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const key = JSON.stringify(filters)
  const expanded = all?.key === key ? all : null
  const items = view === 'sweeps' ? expanded?.sweeps ?? overview.recent_sweeps : view === 'runs' ? expanded?.runs ?? overview.recent_runs : expanded?.conversations ?? overview.recent_conversations
  const pageKey = `${key}:${view}`
  const page = pageSelection.key === pageKey ? Math.min(pageSelection.page, Math.max(0, Math.ceil(items.length / 20) - 1)) : 0
  const visibleItems = items.slice(page * 20, page * 20 + 20)
  const loadAll = async () => {
    setBusy(true); setError('')
    try {
      const snapshot = await getPhoneSnapshot(filters, overview.as_of)
      const projection = projectObservations(snapshot.records, { filtered: snapshot.filtered, selected_record_keys: snapshot.selected_record_keys })
      const map = (values: typeof projection.conversations | typeof projection.sweeps) => values.map(value => projectedActivity(value, projection)).sort((a, b) => b.timestamp_ms - a.timestamp_ms)
      setAll({ key, asOf: snapshot.as_of, conversations: map(projection.conversations), sweeps: map(projection.sweeps), runs: [...map(projection.attempts), ...legacyActivities(projectLegacyObservations(snapshot.legacy))].sort((a, b) => b.timestamp_ms - a.timestamp_ms) })
    } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not load conversation history.') }
    finally { setBusy(false) }
  }
  return <section className="rounded-xl border bg-card p-5 shadow-sm"><div className="flex flex-wrap items-center justify-between gap-3"><div><h2 className="font-semibold">Previous conversations and runs</h2><p className="mt-1 text-sm text-muted-foreground">{expanded ? 'All histories in a pinned snapshot, 20 rows per page.' : 'Recent activity, up to 20 per view.'} Overall totals always cover the full selection.</p></div><Button variant="outline" disabled={busy || expanded?.asOf === overview.as_of} onClick={() => void loadAll()}>{busy ? 'Loading snapshot…' : expanded ? 'Refresh full history' : 'Load all history'}</Button></div>
    <nav className="mt-4 flex flex-wrap gap-2" aria-label="Phone history views">{[['conversations', 'Conversations'], ['sweeps', 'Sweeps'], ['runs', 'Stage / legacy runs']].map(([value, label]) => <button key={value} onClick={() => setView(value)} className={`rounded-lg px-3 py-2 text-sm ${view === value ? 'bg-slate-900 text-white' : 'bg-muted text-muted-foreground'}`}>{label}</button>)}</nav>
    {error && <p role="alert" className="mt-3 text-sm text-red-700">{error}</p>}
    {!items.length ? <p className="mt-5 text-sm text-muted-foreground">No {view} received in this selection.{view === 'conversations' && overview.coverage.legacy_batch_count > 0 ? ' Historical uploads remain under Stage / legacy runs because their conversation ownership was not reported.' : ''}</p> : <div className="mt-4 space-y-3">{visibleItems.map(item => <article key={JSON.stringify([item.installation_id, item.kind, item.entity_id])} className="rounded-lg border p-4"><div className="flex flex-wrap items-start justify-between gap-3"><div className="min-w-0"><Link className="break-all text-sm font-medium text-blue-700 hover:underline" to={activityLink(item, search)}>{item.kind === 'conversation' ? 'Conversation' : item.kind === 'sweep' ? 'Sweep' : 'Run'} {item.entity_id}</Link><p className="mt-1 text-xs text-muted-foreground">{phoneDate(item.timestamp_ms)} · {item.device?.hardware ?? 'Device unknown'} · {item.source ?? 'Source unknown'} · {item.stage ?? 'All recorded stages'}</p></div><Status value={item.status} /></div><div className="mt-3"><ModelConfiguration configuration={item.configuration} id={item.configuration_id} /></div><p className="mt-3 text-xs text-muted-foreground">{item.event_count} metric events · {item.metric_batch_count} batches · Telemetry {item.completeness}</p></article>)}</div>}
    {expanded && <div className="mt-4 flex flex-wrap items-center justify-between gap-3 text-xs text-muted-foreground"><p>{items.length ? page * 20 + 1 : 0}–{Math.min(items.length, page * 20 + 20)} of {items.length} rows{expanded.asOf !== overview.as_of ? ' · newer overview available' : ''}</p><div className="flex gap-2"><Button variant="outline" disabled={page === 0} onClick={() => setPageSelection({ key: pageKey, page: page - 1 })}>Previous</Button><Button variant="outline" disabled={(page + 1) * 20 >= items.length} onClick={() => setPageSelection({ key: pageKey, page: page + 1 })}>Next</Button></div></div>}
  </section>
}
