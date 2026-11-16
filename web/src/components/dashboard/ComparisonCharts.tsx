import { useEffect, useRef, useState } from "react"
import { getSummary, type Experiment } from "@/lib/benchmarkApi"
import { useMetricFilter } from "@/hooks/useMetricFilter"
import MetricFilter from "./MetricFilter"

type Point = { label: string; value: number }
type Series = { name: string; unit: string; scope: string; source: string; points: Point[] }

export default function ComparisonCharts({ experiments }: { experiments: Experiment[] }) {
  const [series, setSeries] = useState<Series[]>([])
  const [loading, setLoading] = useState(true)
  const loadedRef = useRef(false)

  useEffect(() => {
    let active = true
    if (!loadedRef.current) setLoading(true)
    void Promise.all(experiments.filter(experiment => experiment.result_source === "device_reported")
      .map(async experiment => ({ experiment, summary: await getSummary(experiment.id) })))
      .then(results => {
        if (!active) return
        const grouped = new Map<string, Series>()
        results.forEach(({ experiment, summary }) => {
          summary.metrics.forEach(metric => {
            if (metric.mean === null) return
            const key = JSON.stringify([metric.name, metric.unit, metric.scope, metric.source])
            const current = grouped.get(key) ?? { name: metric.name, unit: metric.unit, scope: metric.scope, source: metric.source, points: [] }
            current.points.push({ label: `${experiment.device?.hardware ?? experiment.model_config.name ?? "Experiment"} · ${experiment.id.slice(-6)}`, value: metric.mean })
            grouped.set(key, current)
          })
        })
        setSeries([...grouped.values()])
        loadedRef.current = true
        setLoading(false)
      })
      .catch(() => { if (active) { setSeries([]); setLoading(false) } })
    return () => { active = false }
  }, [experiments])

  const metricNames = series.map(item => item.name)
  const filter = useMetricFilter(metricNames)
  const visible = series.filter(item => filter.activeNames.has(item.name))

  if (loading) return <p className="text-sm text-muted-foreground">Loading comparison metrics…</p>
  if (!series.length) return <p className="text-sm text-muted-foreground">Add phone voice data to see metrics.</p>

  return <div>
    <div className="mb-5 flex flex-wrap items-center justify-between gap-3"><p className="text-sm text-muted-foreground">Each point is the average for one experiment.</p><MetricFilter metricNames={metricNames} {...filter} /></div>
    {visible.length ? <div className="mt-5 grid gap-4 lg:grid-cols-2">{visible.map(item => <div key={JSON.stringify([item.name, item.unit, item.scope, item.source])}><p className="mb-2 text-xs text-muted-foreground">{item.scope} · {item.source}</p><MetricChart {...item} /></div>)}</div> : <p className="rounded-xl border border-dashed p-8 text-center text-sm text-muted-foreground">No metrics selected. Use the filter to select metrics.</p>}
  </div>
}

function MetricChart({ name, unit, points }: Series) {
  const values = points.map(point => point.value)
  const min = Math.min(...values)
  const max = Math.max(...values)
  const spread = max - min || Math.max(Math.abs(max) * 0.1, 1)
  const width = 520
  const height = 180
  const coords = points.map((point, index) => ({ x: points.length === 1 ? width / 2 : 24 + index * ((width - 48) / (points.length - 1)), y: height - 24 - ((point.value - min) / spread) * (height - 48) }))
  const path = coords.map((point, index) => `${index ? "L" : "M"}${point.x},${point.y}`).join(" ")
  const displayUnit = unit === "percent" || unit === "%" ? "%" : unit
  return <article className="rounded-xl border bg-card p-4 shadow-sm"><div className="flex items-baseline justify-between gap-3"><h3 className="truncate font-medium">{name}</h3><span className="text-xs text-muted-foreground">average · {displayUnit}</span></div><svg viewBox={`0 0 ${width} ${height}`} className="mt-4 h-40 w-full" role="img" aria-label={`${name} average by experiment`}><line x1="24" y1="156" x2="496" y2="156" stroke="currentColor" className="text-slate-200" /><line x1="24" y1="24" x2="24" y2="156" stroke="currentColor" className="text-slate-200" /><path d={path} fill="none" stroke="currentColor" strokeWidth="3" className="text-blue-600" />{coords.map((point, index) => <circle key={`${point.x}-${point.y}`} cx={point.x} cy={point.y} r="5" className="fill-blue-600"><title>{points[index].label}: average {points[index].value.toFixed(2)}{displayUnit === "%" ? "%" : ` ${displayUnit}`}</title></circle>)}</svg><div className="flex justify-between text-xs text-muted-foreground"><span>Min {min.toFixed(2)}</span><span>Max {max.toFixed(2)}</span><span>{points.length} experiments</span></div></article>
}
