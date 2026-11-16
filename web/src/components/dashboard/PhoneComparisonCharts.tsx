import { useMemo, useState } from 'react'
import { metricSeriesKey, type PhoneMetricSeries } from '@/lib/phoneBenchmarkTypes'
import { configurationLabel, metricValue } from '@/lib/phonePresentation'

/** Compare reported sample means only inside compatible device/build/metric boundaries. */
export default function PhoneComparisonCharts({ series }: { series: PhoneMetricSeries[] }) {
  const groups = useMemo(() => {
    const result = new Map<string, PhoneMetricSeries[]>()
    for (const item of series) {
      if (item.mean === null) continue
      const key = JSON.stringify([item.installation_id, item.device, item.stage, item.dataset_revision, item.protocol_revision, item.outcome, item.name, item.unit, item.scope, item.source])
      result.set(key, [...result.get(key) ?? [], item])
    }
    return [...result.entries()]
  }, [series])
  const [selection, setSelection] = useState('')
  const selected = groups.find(([key]) => key === selection) ?? groups[0]
  if (!selected) return null
  const [selectedKey, points] = selected
  const first = points[0]
  const min = Math.min(...points.map(item => item.min ?? item.mean!))
  const max = Math.max(...points.map(item => item.max ?? item.mean!))
  const scale = (value: number) => 5 + (value - min) / (max - min || 1) * 90
  return <div className="mt-5 rounded-xl border p-4"><div className="flex flex-wrap items-center justify-between gap-3"><div><h3 className="text-sm font-semibold">Configuration comparisons</h3><p className="mt-1 text-xs text-muted-foreground">Reported sample means and min–max. Devices, builds, stages, units and sources stay separate.</p></div><select aria-label="Comparison metric" className="max-w-full rounded-md border bg-card px-2 py-2 text-xs sm:max-w-80" value={selectedKey} onChange={event => setSelection(event.target.value)}>{groups.map(([key, items]) => <option key={key} value={key}>{items[0].name} · {items[0].stage ?? 'unknown stage'} · {items[0].scope} · {items[0].source} · {items[0].device?.hardware ?? 'unknown device'} · build {items[0].device?.build ?? 'unknown'} · {items[0].installation_id?.slice(-8) ?? 'unknown installation'} · {items[0].outcome ?? 'outcome unknown'}</option>)}</select></div><p className="mt-3 break-all text-[11px] text-muted-foreground">{first.device?.hardware ?? 'Unknown device'} · iOS {first.device?.os_version ?? 'unknown'} · app {first.device?.app_version ?? 'unknown'} ({first.device?.build ?? 'unknown'}) · installation {first.installation_id ?? 'unknown'} · {first.outcome ?? 'outcome unknown'}</p><div className="mt-4 space-y-4">{points.map(item => <div key={metricSeriesKey(item)}><div className="flex flex-wrap justify-between gap-2 text-xs"><span className="break-all">{configurationLabel(item.configuration, item.configuration_id)}</span><span>{metricValue(item.mean, item.unit)} mean · {item.numeric_count} samples</span></div><div className="relative mt-2 h-5 rounded bg-slate-100" role="img" aria-label={`Configuration ${item.configuration_id ?? 'unknown'}, mean ${item.mean} ${item.unit}, min ${item.min}, max ${item.max}`}><span className="absolute top-2 h-1 rounded bg-blue-200" style={{ left: `${scale(item.min ?? item.mean!)}%`, width: `${scale(item.max ?? item.mean!) - scale(item.min ?? item.mean!)}%` }} /><span className="absolute top-1 h-3 w-3 -translate-x-1/2 rounded-full bg-blue-600" style={{ left: `${scale(item.mean!)}%` }} /></div></div>)}</div><div className="mt-3 flex justify-between text-[11px] text-muted-foreground"><span>{metricValue(min, first.unit)}</span><span>{metricValue(max, first.unit)}</span></div></div>
}
