import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { Search } from 'lucide-react'
import type { PhoneActivity, PhoneFilters, PhoneMetricSeries, PhoneOverview } from '@/lib/phoneBenchmarkTypes'
import { metricSeriesKey, numericSegments } from '@/lib/phoneBenchmarkTypes'
import { activityLink, metricValue, phoneDate } from '@/lib/phonePresentation'

const object = (value: unknown): Record<string, unknown> | null => value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
const text = (value: unknown) => typeof value === 'string' && value ? value : 'Unknown'

export function ModelConfiguration({ configuration, id, compact = false }: { configuration: Record<string, unknown> | null; id?: string | null; compact?: boolean }) {
  return <div className="space-y-3">
    <div className={`grid gap-3 ${compact ? '' : 'sm:grid-cols-2'}`}>
      {(['asr', 'reasoning'] as const).map(purpose => {
        const model = object(configuration?.[purpose])
        return <div key={purpose} className="min-w-0 rounded-lg border bg-muted/20 p-3">
          <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{purpose === 'asr' ? 'ASR model' : 'Reasoning model'}</p>
          <p className="mt-1 break-words text-sm font-medium">{text(model?.name)}</p>
          <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">SHA-256: {text(model?.sha256)}</p>
        </div>
      })}
    </div>
    <details className="text-xs text-muted-foreground"><summary className="cursor-pointer break-all">Configuration: {id ?? 'Unknown / legacy metadata'}</summary>
      {configuration ? <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all rounded bg-muted p-3">{JSON.stringify(configuration, null, 2)}</pre> : <p className="mt-2">Effective model settings, runtime and role metadata were not reported.</p>}
    </details>
  </div>
}
export function Status({ value }: { value: string | null }) {
  return <span className={`inline-flex rounded-full px-2.5 py-1 text-xs font-medium ${value === 'running' || value === 'active' ? 'bg-blue-50 text-blue-700' : value === 'succeeded' ? 'bg-emerald-50 text-emerald-700' : value === 'failed' || value === 'interrupted' ? 'bg-amber-50 text-amber-800' : 'bg-slate-100 text-slate-600'}`}>{value ?? 'Outcome unknown'}</span>
}
export function CurrentPhoneRun({ overview, search }: { overview: PhoneOverview | null; search: string }) {
  const conversation = overview?.current_conversation
  const current = conversation && (conversation.status === 'active' || conversation.status === 'running') ? conversation : [conversation, overview?.recent_runs[0]].filter((item): item is PhoneActivity => !!item).sort((a, b) => b.timestamp_ms - a.timestamp_ms)[0]
  const metrics = current?.metrics.filter(item => item.numeric_count > 0).slice(0, 4) ?? []
  return <section className="rounded-xl border border-blue-200 bg-card p-5 shadow-sm" aria-label="Current or latest phone run">
    <div className="flex flex-wrap items-start justify-between gap-3"><div><p className="text-xs font-medium uppercase tracking-wide text-blue-700">Phone activity</p><h2 className="mt-1 text-lg font-semibold">Current / latest run</h2></div>{current && <Status value={current.status} />}</div>
    {!overview ? <p className="mt-4 text-sm text-muted-foreground">Loading received phone activity…</p> : !current ? <p className="mt-4 text-sm text-muted-foreground">No phone conversations or runs have been received in this selection.</p> : <>
      <div className="mt-3 flex flex-wrap justify-between gap-3 text-sm"><div><Link className="break-all font-medium text-blue-700 hover:underline" to={activityLink(current, search)}>{current.kind === 'conversation' ? 'Conversation' : 'Run'} {current.entity_id}</Link><p className="mt-1 text-xs text-muted-foreground">{current.device?.hardware ?? 'Device unknown'} · {current.source ?? 'Source unknown'} · {current.stage ?? 'Conversation stages'}</p></div><div className="text-xs text-muted-foreground"><p>Last activity: {phoneDate(current.timestamp_ms)}</p><p className="mt-1">Telemetry: {current.completeness} · {current.event_count} events</p></div></div>
      <div className="mt-4"><ModelConfiguration configuration={current.configuration} id={current.configuration_id} /></div>
      <h3 className="mt-5 text-sm font-medium">Metrics so far for this {current.kind === 'conversation' ? 'conversation' : 'run'}</h3>
      {metrics.length ? <div className="mt-3 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">{metrics.map(metric => <div key={metricSeriesKey(metric)} className="rounded-lg border p-3"><p className="break-words text-xs text-muted-foreground">{metric.name}</p><p className="mt-1 text-xl font-semibold">{metricValue(metric.latest, metric.unit)}</p><p className="mt-1 text-[11px] text-muted-foreground">Latest · {metric.stage ?? 'stage unknown'} · {metric.scope} · {metric.source}</p></div>)}</div> : <p className="mt-2 text-sm text-muted-foreground">No numeric readings received yet. Execution status and telemetry completeness are tracked separately.</p>}
      <Link to={activityLink(current, search)} className="mt-4 inline-block text-sm font-medium text-blue-700 hover:underline">Inspect all metrics, turns and attempts →</Link>
    </>}
  </section>
}

export function PhoneFilterBar({ filters, update }: { filters: PhoneFilters; update: (key: keyof PhoneFilters, value: string) => void }) {
  const field = 'mt-1 w-full rounded-md border bg-white px-2 py-2 text-xs'
  const dateValue = (value: number | undefined) => { if (value === undefined) return ''; const date = new Date(value); return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}` }
  return <div className="rounded-xl border bg-card p-4">
    <div className="flex flex-wrap items-center justify-between gap-2"><p className="text-sm font-medium">Selected phone dataset</p><div className="flex gap-3 text-xs"><button className="text-blue-700 hover:underline" onClick={() => { update('since_ms', ''); update('until_ms', '') }}>All retained dates</button><button className="text-blue-700 hover:underline" onClick={() => { const today = new Date(); today.setHours(0, 0, 0, 0); update('since_ms', String(today.getTime())); update('until_ms', '') }}>Today</button></div></div>
    <div className="mt-3 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <label className="text-xs text-muted-foreground">From<input aria-label="From date" type="date" className={field} value={dateValue(filters.since_ms)} onChange={event => update('since_ms', event.target.value ? String(new Date(`${event.target.value}T00:00:00`).getTime()) : '')} /></label>
      <label className="text-xs text-muted-foreground">Through<input aria-label="Through date" type="date" className={field} value={dateValue(filters.until_ms)} onChange={event => update('until_ms', event.target.value ? String(new Date(`${event.target.value}T23:59:59.999`).getTime()) : '')} /></label>
      <label className="text-xs text-muted-foreground">Device installation<input className={field} placeholder="All installations" value={filters.installation_id ?? ''} onChange={event => update('installation_id', event.target.value)} /></label>
      <label className="text-xs text-muted-foreground">Configuration<input className={field} placeholder="All configurations" value={filters.configuration_id ?? ''} onChange={event => update('configuration_id', event.target.value)} /></label>
      <label className="text-xs text-muted-foreground">Sweep<input className={field} placeholder="All sweeps" value={filters.sweep_id ?? ''} onChange={event => update('sweep_id', event.target.value)} /></label>
      <label className="text-xs text-muted-foreground">Conversation<input className={field} placeholder="All conversations" value={filters.conversation_id ?? ''} onChange={event => update('conversation_id', event.target.value)} /></label>
      <label className="text-xs text-muted-foreground">Collection source<select aria-label="Collection source" className={field} value={filters.source ?? ''} onChange={event => update('source', event.target.value)}><option value="">All sources</option>{['manual', 'sweep', 'acceptance'].map(value => <option key={value}>{value}</option>)}</select></label>
      <label className="text-xs text-muted-foreground">Stage<select aria-label="Stage" className={field} value={filters.stage ?? ''} onChange={event => update('stage', event.target.value)}><option value="">All stages</option>{['transcription', 'reasoning', 'loading', 'warmup'].map(value => <option key={value}>{value}</option>)}</select></label>
      <label className="text-xs text-muted-foreground">Execution outcome<select aria-label="Execution outcome" className={field} value={filters.outcome ?? ''} onChange={event => update('outcome', event.target.value)}><option value="">All outcomes</option>{['planned', 'active', 'running', 'succeeded', 'failed', 'cancelled', 'interrupted', 'skipped'].map(value => <option key={value}>{value}</option>)}</select></label>
    </div><p className="mt-3 text-xs text-muted-foreground">Filters apply to received data, overview totals, history and exports. Unreported legacy attribution remains unknown.</p>
  </div>
}

export function PhoneProgress({ overview }: { overview: PhoneOverview }) {
  const counts = overview.counts
  return <><div className="mt-4 grid gap-3 sm:grid-cols-3 xl:grid-cols-6">{([
    ['Explicit conversations', counts.conversations.total], ['Explicit turns', counts.turns.total], ['Stage attempts', counts.attempts.total], ['Configurations', counts.configurations], ['Planned / observed jobs', counts.jobs.total], ['Received metric batches', overview.coverage.metric_batch_count],
  ] as const).map(([label, value]) => <div key={label} className="rounded-lg border bg-muted/20 p-3"><p className="text-xs text-muted-foreground">{label}</p><p className="mt-1 text-xl font-semibold tabular-nums">{value.toLocaleString()}</p></div>)}</div>
    <div className="mt-4 flex flex-wrap gap-4 text-xs">{(['conversations', 'turns', 'attempts', 'jobs'] as const).map(kind => <div key={kind}><span className="font-medium capitalize">{kind}: </span>{Object.entries(counts[kind].outcomes).length ? Object.entries(counts[kind].outcomes).map(([status, total]) => `${total} ${status}`).join(' · ') : 'No explicit outcomes'}</div>)}</div>
    {overview.coverage.legacy_batch_count > 0 && <p className="mt-3 rounded-lg bg-amber-50 p-3 text-xs text-amber-900">Includes {overview.coverage.legacy_batch_count} legacy batches. Their conversation count and unreported execution outcomes are unknown; they are not included in explicit conversation totals.</p>}
    <details className="mt-4 text-xs text-muted-foreground"><summary className="cursor-pointer">Data health · {overview.coverage.unresolved_record_count} unresolved records · {overview.coverage.finalization.dropped_events} reported dropped events</summary><p className="mt-2">Telemetry finalizations: {overview.coverage.finalization.complete} complete · {overview.coverage.finalization.partial} partial · {overview.coverage.finalization.unknown} unknown. Finished execution does not imply complete telemetry. This view shows server receipts; phone upload queues and offline activity are not visible until reported.</p></details>
  </>
}
function category(name: string) {
  if (/duration|latency|time|rtf/.test(name)) return 'Timing'
  if (/cpu|memory|ram|resident|footprint|gpu/.test(name)) return 'CPU, memory and resources'
  if (/battery|thermal|charging/.test(name)) return 'Battery and thermal'
  if (/power|energy|joule|watt/.test(name)) return 'Power and energy'
  return 'Counts, status and other metrics'
}
export function PhoneSeries({ series, samples }: { series: PhoneMetricSeries[]; samples?: Map<string, Array<{ timestamp: number; value: number | null }>> }) {
  const [search, setSearch] = useState('')
  const groups = useMemo(() => {
    const result = new Map<string, PhoneMetricSeries[]>()
    for (const item of series) {
      if (search && ![item.name, item.source, item.scope, item.stage, item.configuration_id, item.installation_id].join(' ').toLowerCase().includes(search.toLowerCase())) continue
      const name = category(item.name)
      result.set(name, [...result.get(name) ?? [], item])
    }
    const order = ['Timing', 'CPU, memory and resources', 'Battery and thermal', 'Power and energy', 'Counts, status and other metrics']
    return new Map([...result.entries()].sort((a, b) => order.indexOf(a[0]) - order.indexOf(b[0])))
  }, [series, search])
  return <div className="mt-5">
    <div className="flex flex-wrap items-center justify-between gap-3"><p className="text-sm text-muted-foreground">{series.length} compatible metric series · numeric values summarize reported samples</p><label className="flex items-center gap-2 rounded-lg border px-3 py-2"><Search className="size-4 text-muted-foreground" /><input aria-label="Search all metrics" className="w-48 bg-transparent text-sm outline-none" placeholder="Search all metrics…" value={search} onChange={event => setSearch(event.target.value)} /></label></div>
    {!series.length && <p className="mt-4 text-sm text-muted-foreground">No metric series have been received in this selection.</p>}
    {series.length > 0 && !groups.size && <p className="mt-4 text-sm text-muted-foreground">No metric series match this search.</p>}
    <div className="mt-4 space-y-3">{[...groups.entries()].map(([name, items]) => <details key={name} open={name === 'Timing' || !!search} className="rounded-lg border"><summary className="cursor-pointer px-4 py-3 text-sm font-medium">{name} <span className="ml-1 text-xs font-normal text-muted-foreground">({items.length})</span></summary><div className="space-y-3 border-t p-3">{items.map(item => <details key={metricSeriesKey(item)} className="rounded-lg border bg-muted/10 p-3">
      <summary className="cursor-pointer list-none"><div className="flex flex-wrap items-start justify-between gap-3"><div className="min-w-0"><p className="break-words text-sm font-medium">{item.name}</p><p className="mt-1 text-[11px] text-muted-foreground">{item.stage ?? 'Stage unknown'} · {item.scope} · {item.source} · {item.unit}</p><p className="mt-1 break-all text-[11px] text-muted-foreground">{item.device?.hardware ?? 'Device unknown'} · installation {item.installation_id ?? 'unknown'} · config {item.configuration_id ?? 'unknown'} · {item.outcome ?? 'outcome unknown'}</p></div><div className="min-w-0 max-w-full break-all text-right"><p className="text-sm font-semibold tabular-nums">{metricValue(item.latest, item.unit)}</p><p className="mt-1 text-[11px] text-muted-foreground">Latest · {phoneDate(item.latest_timestamp_ms)}</p></div></div>
      <div className="mt-3 flex flex-wrap gap-x-5 gap-y-1 text-xs">{item.numeric_count > 0 || item.unavailable_count > 0 ? <><span>Mean {metricValue(item.mean, item.unit)}</span><span>Min {metricValue(item.min, item.unit)}</span><span>Max {metricValue(item.max, item.unit)}</span><span>{item.numeric_count} numeric</span><span>{item.unavailable_count} unavailable</span></> : <span>{item.observation_count} categorical observations</span>}<span className="text-blue-700">Details ↓</span></div></summary>
      <div className="mt-3 space-y-3 border-t pt-3 text-xs"><p>Observations: {item.observation_count} · Dataset revision: {item.dataset_revision ?? 'unknown'} · Protocol revision: {item.protocol_revision ?? 'unknown'}</p>
        {Object.keys(item.status_counts ?? {}).length > 0 && <p>Status observations: {Object.entries(item.status_counts).map(([status, count]) => `${status}: ${count}`).join(' · ')}</p>}
        {item.unavailable_reasons.length > 0 && <p className="text-amber-800">Unavailable: {item.unavailable_reasons.join('; ')}</p>}
        {samples?.has(metricSeriesKey(item)) && <MetricTimeline points={samples.get(metricSeriesKey(item))!} name={item.name} unit={item.unit} />}
        <ModelConfiguration configuration={item.configuration} id={item.configuration_id} />
      </div>
    </details>)}</div></details>)}</div>
    <p className="mt-4 text-xs text-muted-foreground">No readings are available for unsupported GPU utilization, verified whole-device watts or joules unless a provider explicitly reports them. Battery percentages are not energy measurements. Sample means and sampled peaks do not imply time-weighted values or instantaneous peaks.</p>
  </div>
}
export function MetricTimeline({ points, name, unit }: { points: Array<{ timestamp: number; value: number | null }>; name: string; unit: string }) {
  const segments = numericSegments(points)
  const valid = segments.flat()
  if (!valid.length) return <p>No numeric samples available for this series.</p>
  const min = Math.min(...valid.map(point => point.value)), max = Math.max(...valid.map(point => point.value))
  const start = Math.min(...points.map(point => point.timestamp)), end = Math.max(...points.map(point => point.timestamp))
  const x = (timestamp: number) => 20 + (timestamp - start) / (end - start || 1) * 560
  const y = (value: number) => 135 - (value - min) / (max - min || 1) * 115
  return <div><svg viewBox="0 0 600 160" className="h-40 w-full" role="img" aria-label={`${name} sample history; lines break at unavailable samples`}><line x1="20" x2="580" y1="135" y2="135" stroke="#cbd5e1" />{segments.map((segment, index) => <g key={index}><path d={segment.map((point, i) => `${i ? 'L' : 'M'}${x(point.timestamp)},${y(point.value)}`).join(' ')} stroke="#2563eb" strokeWidth="2" fill="none" />{segment.map((point, i) => <circle key={i} cx={x(point.timestamp)} cy={y(point.value)} r="3" fill="#2563eb"><title>{phoneDate(point.timestamp)}: {metricValue(point.value, unit)}</title></circle>)}</g>)}</svg><p className="text-xs text-muted-foreground">{phoneDate(start)} – {phoneDate(end)} · {valid.length} numeric samples · {points.length - valid.length} gaps</p><details className="mt-2"><summary className="cursor-pointer">Raw sample history</summary><div className="mt-2 max-h-56 overflow-auto"><table className="w-full text-left"><thead><tr><th>Collected</th><th>Value</th></tr></thead><tbody>{[...points].sort((a, b) => a.timestamp - b.timestamp).map((point, i) => <tr key={i}><td className="py-1">{phoneDate(point.timestamp)}</td><td>{metricValue(point.value, unit)}</td></tr>)}</tbody></table></div></details></div>
}
