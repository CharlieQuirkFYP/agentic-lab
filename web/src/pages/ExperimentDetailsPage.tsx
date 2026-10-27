import { useEffect, useState } from "react"
import { Link, NavLink, useParams } from "react-router-dom"
import { ArrowLeft, Download, FlaskConical, RefreshCw } from "lucide-react"
import { Button } from "@/components/ui/button"
import MetricFilter from "@/components/dashboard/MetricFilter"
import { useMetricFilter } from "@/hooks/useMetricFilter"
import { exportExperiment, getExperiment, getMetrics, getRuns, getSummary, type Experiment, type MetricBatch, type MetricSummary, type RunPage, type Summary } from "@/lib/benchmarkApi"

const navClass = ({ isActive }: { isActive: boolean }) => isActive ? "block rounded-lg bg-slate-900 px-3 py-2 text-sm font-medium text-white" : "block rounded-lg px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-muted"
const date = (value?: string | number) => value ? new Date(value).toLocaleString() : "—"

export default function ExperimentDetailsPage() {
  const { id = "" } = useParams()
  const [experiment, setExperiment] = useState<Experiment | null>(null)
  const [summary, setSummary] = useState<Summary | null>(null)
  const [runs, setRuns] = useState<RunPage | null>(null)
  const [batches, setBatches] = useState<MetricBatch[]>([])
  const [error, setError] = useState("")

  const load = async () => {
    try {
      const [item, nextSummary, nextRuns, metrics] = await Promise.all([getExperiment(id), getSummary(id), getRuns(id), getMetrics(id)])
      setExperiment(item); setSummary(nextSummary); setRuns(nextRuns); setBatches(metrics.batches); setError("")
    } catch (err) { setError(err instanceof Error ? err.message : "Could not load experiment details.") }
  }

  useEffect(() => { void load() }, [id])

  const download = async () => {
    const data = await exportExperiment(id)
    const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: "application/json" }))
    const anchor = document.createElement("a"); anchor.href = url; anchor.download = `${id}.json`; anchor.click(); URL.revokeObjectURL(url)
  }

  const allMetrics = summary?.metrics ?? []
  const filter = useMetricFilter(allMetrics.map(metric => metric.name))
  const visibleMetrics = allMetrics.filter(metric => filter.activeNames.has(metric.name))
  const chartMetrics = visibleMetrics.filter(metric => metric.numeric_count > 0).map(metric => ({
    ...metric,
    points: batches.flatMap(batch => batch.events.filter(event => event.name === metric.name && typeof event.value === "number").map(event => ({ label: `${batch.run_id} · ${event.sequence}`, value: event.value as number, timestamp: event.timestamp_ms }))),
  }))

  return <main className="min-h-screen bg-[#f8fafc] text-foreground"><div className="mx-auto grid min-h-screen max-w-360 lg:grid-cols-[232px_1fr]">
    <aside className="border-b bg-card px-4 py-5 lg:border-r lg:border-b-0"><div className="flex items-center gap-3 px-2"><div className="grid size-9 place-items-center rounded-lg bg-slate-900 text-white"><FlaskConical className="size-5" /></div><div><p className="font-semibold">Agentic Lab</p><p className="text-xs text-muted-foreground">Benchmark workspace</p></div></div><nav className="mt-8 space-y-1"><NavLink to="/dashboard" className={navClass}>Dashboard &amp; Metrics</NavLink><NavLink to="/benchmark-testing" className={navClass}>Benchmark Testing</NavLink><NavLink to="/downloads" className={navClass}>Download Manager</NavLink></nav></aside>
    <div className="min-w-0"><header className="flex flex-wrap items-center justify-between gap-3 border-b bg-card px-5 py-4 sm:px-8"><div><Link to="/dashboard" className="flex items-center gap-1 text-sm text-blue-700"><ArrowLeft className="size-4" /> Back to dashboard</Link><h1 className="mt-2 text-xl font-semibold">Experiment details</h1><p className="mt-1 font-mono text-xs text-muted-foreground">{id}</p></div><div className="flex gap-2"><Button variant="outline" onClick={() => void load()}><RefreshCw /> Refresh</Button><Button variant="outline" onClick={() => void download()}><Download /> JSON export</Button></div></header>
      <div className="space-y-6 px-5 py-7 sm:px-8">{error && <p className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-700">{error}</p>}{experiment && <>
        <section className="grid gap-4 sm:grid-cols-4"><Card label="Status" value={experiment.status} /><Card label="Result source" value={experiment.result_source} /><Card label="Runs" value={String(summary?.run_count ?? 0)} /><Card label="Metric batches" value={String(batches.length)} /></section>
        <section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Configuration</h2><dl className="mt-4 grid gap-4 text-sm sm:grid-cols-2 lg:grid-cols-3"><Entry label="Use case" value={experiment.use_case} /><Entry label="Dataset" value={experiment.dataset} /><Entry label="Model" value={experiment.model_config.name} /><Entry label="Quantization" value={experiment.model_config.quantization} /><Entry label="Runtime" value={experiment.model_config.runtime} /><Entry label="Environment" value={experiment.environment_profile.name} /><Entry label="Created" value={date(experiment.created_at)} /><Entry label="Started" value={date(experiment.started_at)} /><Entry label="Completed" value={date(experiment.completed_at)} /></dl></section>
        <section className="rounded-xl border bg-card p-5 shadow-sm"><div className="flex flex-wrap items-center justify-between gap-3"><div><h2 className="font-semibold">Metrics and timeline</h2><p className="mt-1 text-sm text-muted-foreground">The filter is shared with the dashboard.</p></div><MetricFilter metricNames={allMetrics.map(metric => metric.name)} {...filter} /></div>{visibleMetrics.length ? <div className="mt-4 grid gap-3 sm:grid-cols-2">{visibleMetrics.map(metric => <SummaryCard key={`${metric.name}-${metric.unit}-${metric.source}`} metric={metric} />)}</div> : <p className="mt-4 text-sm text-muted-foreground">No metrics selected. Use the filter to select metrics.</p>}</section>
        <section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Metric charts over time</h2>{chartMetrics.length ? <div className="mt-4 grid gap-4 lg:grid-cols-2">{chartMetrics.map(metric => <MetricChart key={`${metric.name}-${metric.unit}`} name={metric.name} unit={metric.unit} points={metric.points} />)}</div> : <p className="mt-4 text-sm text-muted-foreground">No numeric metric data available.</p>}</section>
        <section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Runs ({runs?.total ?? 0})</h2>{runs?.runs.length ? <div className="mt-4 space-y-3">{runs.runs.map(run => <div key={run.run_id} className="rounded-lg border p-4"><div className="flex flex-wrap justify-between gap-2"><span className="font-mono text-sm">{run.run_id}</span><span className="text-sm text-muted-foreground">{run.event_count} events · {run.batch_count} batches</span></div><p className="mt-2 text-xs text-muted-foreground">{date(run.first_timestamp_ms)} – {date(run.last_timestamp_ms)}</p></div>)}</div> : <p className="mt-4 text-sm text-muted-foreground">No metric runs uploaded.</p>}</section>
        <section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Raw metric batches</h2>{batches.length ? <div className="mt-4 space-y-4">{batches.map(batch => <details key={`${batch.run_id}-${batch.sequence_number}`} className="rounded-lg border p-4"><summary className="cursor-pointer font-mono text-sm">{batch.run_id} · batch {batch.sequence_number} · {batch.events.length} events</summary><pre className="mt-3 overflow-x-auto rounded bg-slate-950 p-3 text-xs text-slate-100">{JSON.stringify(batch, null, 2)}</pre></details>)}</div> : <p className="mt-4 text-sm text-muted-foreground">No raw metric batches uploaded.</p>}</section>
        <section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Benchmark result</h2><pre className="mt-4 overflow-x-auto rounded bg-slate-950 p-4 text-xs text-slate-100">{JSON.stringify(experiment.result ?? summary?.result ?? null, null, 2)}</pre></section>
      </>}</div></div>
  </div></main>
}

function Card({ label, value }: { label: string; value: string }) { return <div className="rounded-xl border bg-card p-5 shadow-sm"><p className="text-sm text-muted-foreground">{label}</p><p className="mt-2 text-xl font-semibold capitalize">{value}</p></div> }
function Entry({ label, value }: { label: string; value?: string | number }) { return <div><dt className="text-xs text-muted-foreground">{label}</dt><dd className="mt-1 font-medium">{value || "—"}</dd></div> }
function SummaryCard({ metric }: { metric: MetricSummary }) { return <div className="rounded-lg border p-4"><p className="font-medium">{metric.name}</p><p className="mt-1 text-xl font-semibold">{metric.mean == null ? "Unavailable" : `${metric.mean} ${metric.unit}`}</p><p className="mt-1 text-xs text-muted-foreground">min {metric.min ?? "—"} · max {metric.max ?? "—"} · {metric.observation_count} observations</p></div> }
function MetricChart({ name, unit, points }: { name: string; unit: string; points: { label: string; value: number; timestamp: number }[] }) { const values = points.map(point => point.value); const min = Math.min(...values); const max = Math.max(...values); const spread = max - min || Math.max(Math.abs(max) * 0.1, 1); const first = Math.min(...points.map(point => point.timestamp)); const last = Math.max(...points.map(point => point.timestamp)); const timeSpread = last - first || 1; const width = 520; const height = 180; const coords = points.map((point, index) => ({ x: points.length === 1 ? width / 2 : (24 + ((point.timestamp - first) / timeSpread) * (width - 48)) || 24 + index * ((width - 48) / (points.length - 1)), y: height - 24 - ((point.value - min) / spread) * (height - 48) })); const path = coords.map((point, index) => `${index ? "L" : "M"}${point.x},${point.y}`).join(" "); return <article className="rounded-xl border bg-card p-4 shadow-sm"><div className="flex items-baseline justify-between gap-3"><h3 className="truncate font-medium">{name}</h3><span className="text-xs text-muted-foreground">{unit} · over time</span></div><svg viewBox={`0 0 ${width} ${height}`} className="mt-4 h-40 w-full" role="img" aria-label={`${name} over experiment time`}><line x1="24" y1="156" x2="496" y2="156" stroke="currentColor" className="text-slate-200" /><line x1="24" y1="24" x2="24" y2="156" stroke="currentColor" className="text-slate-200" /><path d={path} fill="none" stroke="currentColor" strokeWidth="3" className="text-blue-600" />{coords.map((point, index) => <circle key={`${point.x}-${point.y}`} cx={point.x} cy={point.y} r="5" className="fill-blue-600"><title>{points[index].label}: {points[index].value} {unit} at {new Date(points[index].timestamp).toLocaleString()}</title></circle>)}</svg><div className="flex justify-between text-xs text-muted-foreground"><span>Min {min}</span><span>Max {max}</span><span>{points.length} observations</span></div></article> }
