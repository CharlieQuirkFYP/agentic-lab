import { useEffect, useMemo, useState } from 'react'
import { Link, useParams, useSearchParams } from 'react-router-dom'
import { ArrowLeft, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { ModelConfiguration, PhoneSeries, Status } from '@/components/dashboard/PhoneMetrics'
import { activityLink, metricValue, phoneDate } from '@/lib/phonePresentation'
import PhoneExport from '@/components/dashboard/PhoneExport'
import { usePhoneOverview } from '@/hooks/usePhoneOverview'
import { getPhoneSnapshot } from '@/lib/phoneBenchmarkApi'
import { legacyConfiguration, legacyRunRecords, projectedActivity, projectedSeries } from '@/lib/phoneProjection'
import { parsePhoneFilters, type LegacyPhoneObservation } from '@/lib/phoneBenchmarkTypes'
import { projectLegacyObservations, projectObservations } from '../../../shared/phone-benchmarks/index.ts'
import type { EntityProjection, ObservationProjection } from '../../../shared/phone-benchmarks/types.ts'

export default function PhoneDetailsPage() {
  const { kind = 'conversations', id = '' } = useParams()
  const [params] = useSearchParams()
  const filters = parsePhoneFilters(params)
  if (kind === 'conversations') filters.conversation_id = id
  if (kind === 'sweeps') filters.sweep_id = id
  if (kind === 'runs' && params.get('detail_conversation_id')) filters.conversation_id = params.get('detail_conversation_id')!
  const phone = usePhoneOverview(filters)
  const [loaded, setLoaded] = useState<{ key: string; snapshot: Awaited<ReturnType<typeof getPhoneSnapshot>> } | null>(null)
  const [error, setError] = useState('')
  const filterKey = JSON.stringify(filters)
  const snapshot = loaded?.key === filterKey ? loaded.snapshot : null
  const asOf = phone.overview?.as_of
  useEffect(() => {
    if (!asOf) return
    const controller = new AbortController()
    let active = true
    void getPhoneSnapshot(JSON.parse(filterKey), asOf, controller.signal).then(value => {
      if (active) { setLoaded({ key: filterKey, snapshot: value }); setError('') }
    }).catch(reason => { if (active) setError(reason instanceof Error ? reason.message : 'Could not read phone snapshot.') })
    return () => { active = false; controller.abort() }
  }, [filterKey, asOf])
  const projection = useMemo(() => snapshot ? projectObservations(snapshot.records, { filtered: snapshot.filtered, selected_record_keys: snapshot.selected_record_keys }) : null, [snapshot])
  const entities = kind === 'conversations' ? projection?.conversations : kind === 'sweeps' ? projection?.sweeps : projection?.attempts
  const entity = entities?.find(item => item.entity_id === id && (!filters.installation_id || item.installation_id === filters.installation_id))
  const activity = entity && projection ? projectedActivity(entity, projection) : null
  const series = entity && projection ? projectedSeries(entity.metric_groups, projection) : null
  const legacyExperimentID = params.get('legacy_experiment_id') ?? undefined
  const legacyRecords = useMemo(() => snapshot && kind === 'runs' && !entity ? legacyRunRecords(snapshot.legacy, id, filters.installation_id ?? null, legacyExperimentID) : [], [snapshot, kind, entity, id, filters.installation_id, legacyExperimentID])
  const returnSearch = params.get('return') ?? ''
  const title = kind === 'conversations' ? 'Conversation metrics' : kind === 'sweeps' ? 'Sweep metrics' : 'Run metrics'
  const configuration = activity?.configuration ?? (legacyRecords.length ? legacyConfiguration(legacyRecords) : null)
  const turns = projection?.turns.filter(turn => turn.installation_id === entity?.installation_id && (kind === 'conversations' ? turn.context.conversation_id === id : kind === 'sweeps' ? turn.context.sweep_id === id : turn.entity_id === entity?.context.turn_id)) ?? []
  const conversations = kind === 'sweeps' ? projection?.conversations.filter(conversation => conversation.context.sweep_id === id && conversation.installation_id === entity?.installation_id) ?? [] : []
  const jobs = kind === 'sweeps' ? projection?.jobs.filter(job => job.context.sweep_id === id && job.installation_id === entity?.installation_id) ?? [] : []
  return <main className="min-h-screen bg-[#f8fafc] px-4 py-6 text-foreground sm:px-8"><div className="mx-auto max-w-6xl">
    <header className="flex flex-wrap items-start justify-between gap-4"><div><Link to={`/dashboard?${returnSearch}`} className="flex items-center gap-1 text-sm text-blue-700"><ArrowLeft className="size-4" /> Back to selected overview</Link><h1 className="mt-3 text-2xl font-semibold">{title}</h1><p className="mt-2 break-all font-mono text-xs text-muted-foreground">{id}</p></div><div className="flex flex-wrap gap-2"><Button variant="outline" onClick={phone.refresh}><RefreshCw className="size-4" /> Refresh</Button>{kind !== 'runs' && <PhoneExport filters={filters} asOf={snapshot?.as_of} />}</div></header>
    <div className="mt-6 space-y-5">
      {(error || phone.error) && <p role="alert" className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-700">{error || phone.error}</p>}
      {!snapshot && !error && <p className="text-sm text-muted-foreground">Loading all records in a consistent server snapshot…</p>}
      {snapshot && !entity && !legacyRecords.length && <p className="rounded-xl border bg-card p-5 text-sm text-muted-foreground">This entity has no received records in the selected installation and filters. Missing records do not establish an execution outcome.</p>}
      {(entity || legacyRecords.length > 0) && <>
        <section className="rounded-xl border bg-card p-5 shadow-sm"><div className="flex flex-wrap items-center justify-between gap-3"><h2 className="font-semibold">Recorded configuration and outcome</h2><Status value={entity?.status ?? null} /></div><p className="mt-2 text-xs text-muted-foreground">{activity?.device?.hardware ?? legacyRecords[0]?.device?.hardware ?? 'Device unknown'} · Installation {filters.installation_id ?? 'unknown'} · Telemetry {entity?.completeness ?? 'partial / legacy finalization unavailable'}</p><div className="mt-4"><ModelConfiguration configuration={configuration} id={entity?.context.configuration_id} /></div><div className="mt-4 grid gap-3 text-xs sm:grid-cols-3"><p>Started: {phoneDate(entity?.started_at_ms)}</p><p>Finished: {phoneDate(entity?.finished_at_ms)}</p><p>Recorded duration: {metricValue(entity?.duration_ms, 'ms')}</p></div><p className="mt-3 text-xs text-muted-foreground">Execution completion does not measure transcript accuracy or incident-task success. Human review, loading and speech playback are outside an attempt duration unless its recorded scope says otherwise.</p></section>
        {entity && projection && series && <>
          {kind === 'sweeps' && <section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Sweep progress</h2><p className="mt-2 text-sm">{jobs.length} observed jobs · {typeof entity.data.planned_jobs === 'number' ? entity.data.planned_jobs : 'Unknown'} originally planned · {conversations.length} observed conversation executions</p><div className="mt-3 flex flex-wrap gap-3 text-xs">{['planned', 'running', 'succeeded', 'failed', 'cancelled', 'interrupted', 'skipped'].map(status => <span key={status}>{jobs.filter(job => job.status === status).length} {status}</span>)}</div><details className="mt-4 text-xs"><summary className="cursor-pointer">Job outcomes and case identifiers</summary><div className="mt-3 space-y-2">{jobs.map(job => <p key={job.entity_id} className="break-all">{job.entity_id} · case {typeof job.data.case_id === 'string' ? job.data.case_id : 'unknown'} · repetition {typeof job.data.repetition === 'number' ? job.data.repetition : 'unknown'} · {job.status ?? 'outcome unknown'}</p>)}</div></details></section>}
          <section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">{title} overview</h2><PhoneSeries {...series} /></section>
          {!!conversations.length && <section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Conversation executions ({conversations.length})</h2><div className="mt-4 space-y-3">{conversations.map(conversation => <div key={conversation.entity_id} className="flex flex-wrap justify-between gap-3 rounded-lg border p-3"><Link className="break-all text-sm text-blue-700" to={activityLink(projectedActivity(conversation, projection), returnSearch)}>{conversation.entity_id}</Link><Status value={conversation.status} /></div>)}</div></section>}
          {kind !== 'runs' && <section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Turns and stage attempts ({turns.length} turns)</h2>{turns.length ? <div className="mt-4 space-y-4">{turns.map(turn => <details key={turn.entity_id} open className="rounded-lg border p-4"><summary className="cursor-pointer text-sm font-medium">Turn {turn.turn_index == null ? turn.entity_id : turn.turn_index} <span className="ml-2"><Status value={turn.status} /></span></summary><p className="mt-2 text-xs text-muted-foreground">Input kind: {typeof turn.data.input_kind === 'string' ? turn.data.input_kind : 'unknown'} · Telemetry {turn.completeness} · {turn.attempts.length} stage attempts</p><div className="mt-3 space-y-3">{turn.attempts.map(attempt => <AttemptMetrics key={attempt.entity_id} entity={attempt} projection={projection} />)}{!turn.attempts.length && <p className="text-xs text-muted-foreground">No attempt lifecycle records received. This does not imply a typed turn or a failed transcription.</p>}</div></details>)}</div> : <p className="mt-4 text-sm text-muted-foreground">No explicit turn records received. Any unlinked attempt metrics remain visible below.</p>}</section>}
          {kind !== 'runs' && projection.attempts.filter(attempt => attempt.installation_id === entity.installation_id && (kind === 'conversations' ? attempt.context.conversation_id === id : attempt.context.sweep_id === id) && !turns.some(turn => turn.attempts.some(child => child.entity_id === attempt.entity_id))).map(attempt => <AttemptMetrics key={attempt.entity_id} entity={attempt} projection={projection} />)}
          <details className="rounded-xl border bg-card p-5 text-xs"><summary className="cursor-pointer font-medium">Snapshot and unresolved relationships</summary><p className="mt-3 break-all font-mono">{snapshot?.as_of}</p><p className="mt-2">{projection.coverage.unresolved_reference_count} unresolved references · {projection.coverage.dropped_events} reported dropped events · {projection.coverage.completeness} telemetry</p><pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap">{JSON.stringify(projection.unresolved_references, null, 2)}</pre></details>
        </>}
        {legacyRecords.length > 0 && <LegacyRunDetail records={legacyRecords} />}
      </>}
    </div>
  </div></main>
}
function AttemptMetrics({ entity, projection }: { entity: EntityProjection; projection: ObservationProjection }) {
  const series = projectedSeries(entity.metric_groups, projection)
  return <details className="rounded-lg border bg-card p-4"><summary className="cursor-pointer text-sm font-medium"><span className="capitalize">{entity.stage ?? 'Stage unknown'}</span> attempt · <span className="font-mono text-xs">{entity.entity_id}</span> <span className="ml-2"><Status value={entity.status} /></span></summary><p className="mt-3 text-xs text-muted-foreground">Recorded duration {metricValue(entity.duration_ms, 'ms')} · Telemetry {entity.completeness}</p><PhoneSeries {...series} /></details>
}
function LegacyRunDetail({ records }: { records: LegacyPhoneObservation[] }) {
  const legacy = projectLegacyObservations(records)
  const series = projectedSeries(legacy.metric_groups, { raw_metrics: legacy.raw_metrics, configurations: [] })
  const turnIDs = [...new Set(legacy.raw_metrics.map(metric => metric.context.turn_id).filter(Boolean))]
  return <section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Historical linked runs</h2><p className="mt-2 text-sm text-muted-foreground">{records.length} batches linked only by explicit IDs in this installation. Conversation ownership and complete lifecycle records are unavailable; this history is partial.</p><p className="mt-2 text-xs text-muted-foreground">Reported turn IDs: {turnIDs.length ? turnIDs.join(', ') : 'unknown'}. A missing parent run does not establish a typed input or a transcription failure.</p><PhoneSeries {...series} /><details className="mt-5 text-xs"><summary className="cursor-pointer">Original batches and stage attribution</summary><div className="mt-3 space-y-3">{records.map(record => <details key={`${record.batch.experiment_id}:${record.batch.run_id}:${record.batch.sequence_number}`} className="rounded-lg border p-3"><summary className="cursor-pointer break-all">{record.batch.run_id} · batch {record.batch.sequence_number} · {record.batch.events.length} events</summary><pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap break-all">{JSON.stringify(record, null, 2)}</pre></details>)}</div></details></section>
}
