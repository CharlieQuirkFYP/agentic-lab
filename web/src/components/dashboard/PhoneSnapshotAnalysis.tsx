import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { getPhoneSnapshot } from '@/lib/phoneBenchmarkApi'
import { projectedSeries } from '@/lib/phoneProjection'
import { metricValue } from '@/lib/phonePresentation'
import type { PhoneFilters } from '@/lib/phoneBenchmarkTypes'
import { createBenchmarkReport, type BenchmarkReport } from '../../../../shared/phone-benchmarks/index.ts'
import { PhoneSeries } from './PhoneMetrics'

export default function PhoneSnapshotAnalysis({ filters, asOf }: { filters: PhoneFilters; asOf: string }) {
  const [loaded, setLoaded] = useState<{ key: string; report: BenchmarkReport } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const key = JSON.stringify(filters)
  const report = loaded?.key === key ? loaded.report : null
  const load = async () => {
    setBusy(true); setError('')
    try {
      const snapshot = await getPhoneSnapshot(filters, asOf)
      setLoaded({ key, report: createBenchmarkReport(snapshot.records, { as_of: asOf, generated_at_ms: Date.now(), filters, legacy_records: snapshot.legacy, selected_record_keys: snapshot.selected_record_keys, input_already_filtered: true }) })
    } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not load sample history.') }
    finally { setBusy(false) }
  }
  const plotted = report ? projectedSeries(report.metric_groups, { raw_metrics: [...report.projection.raw_metrics, ...report.legacy.raw_metrics], configurations: report.projection.configurations }) : null
  return <details className="mt-5 rounded-lg border p-4"><summary className="cursor-pointer text-sm font-medium">Execution distributions and sample histories</summary><p className="mt-2 text-xs text-muted-foreground">Load a complete selected snapshot for raw time-series charts and execution-duration statistics. Duration distributions use distinct executions; sample readings do not count as additional executions.</p><Button className="mt-3" variant="outline" disabled={busy || report?.input.as_of === asOf} onClick={() => void load()}>{busy ? 'Loading full snapshot…' : report ? 'Refresh analysis snapshot' : 'Load analysis snapshot'}</Button>{error && <p className="mt-2 text-sm text-red-700" role="alert">{error}</p>}{report && <><p className="mt-3 break-all text-xs text-muted-foreground">Snapshot {report.input.as_of}{report.input.as_of !== asOf ? ' · newer overview data available' : ''}</p>{report.duration_groups.length ? <div className="mt-4 space-y-3">{report.duration_groups.map((group, index) => <div key={index} className="rounded-lg border p-3 text-xs"><p className="font-medium">{group.entity_kind} · {group.stage ?? 'all stages'} · {group.outcome ?? 'unknown outcome'}</p><p className="mt-1 break-all text-muted-foreground">{group.installation_id} · config {group.configuration_id ?? 'unknown'} · source {group.source ?? 'unknown'} · dataset {group.dataset_revision ?? 'unknown'} · protocol {group.protocol_revision ?? 'unknown'}</p><p className="mt-2">{group.numeric.count} executions · mean {metricValue(group.numeric.mean, 'ms')} · median {metricValue(group.numeric.median, 'ms')} · min {metricValue(group.numeric.min, 'ms')} · max {metricValue(group.numeric.max, 'ms')}</p><p className="mt-1">p95: {group.numeric.p95 === null ? `Insufficient eligible samples (${group.numeric.p95_eligible_count}/20)` : metricValue(group.numeric.p95, 'ms')} · nearest-rank; successful executions with complete telemetry only</p></div>)}</div> : <p className="mt-3 text-xs text-muted-foreground">No explicit execution-duration records in this snapshot. Legacy samples do not establish completed conversations or eligible latency distributions.</p>}{plotted && <PhoneSeries {...plotted} />}</>}</details>
}
