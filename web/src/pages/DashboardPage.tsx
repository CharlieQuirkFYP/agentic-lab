import { useEffect, useState } from "react"
import { Link, NavLink, useNavigate } from "react-router-dom"
import { Download, FlaskConical, RefreshCw } from "lucide-react"
import { Button } from "@/components/ui/button"
import ComparisonCharts from "@/components/dashboard/ComparisonCharts"
import { exportExperiment, getVoiceMetrics, listExperiments, type Experiment, type MetricBatch, type VoiceMetricsPage } from "@/lib/benchmarkApi"

const navClass = ({ isActive }: { isActive: boolean }) => isActive ? "block rounded-lg bg-slate-900 px-3 py-2 text-sm font-medium text-white" : "block rounded-lg px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-muted"
const formatDate = (value: string) => new Date(value).toLocaleString()

export default function DashboardPage() {
  const [experiments, setExperiments] = useState<Experiment[]>([])
  const [selected, setSelected] = useState<Experiment | null>(null)
  const [voiceMetrics, setVoiceMetrics] = useState<VoiceMetricsPage | null>(null)
  const [error, setError] = useState("")
  const navigate = useNavigate()
  const load = async (id?: string) => { try { const page = await listExperiments(); setExperiments(page.experiments); const next = page.experiments.find(item => item.id === id) ?? page.experiments[0] ?? null; setSelected(next); setVoiceMetrics(await getVoiceMetrics().catch(() => null)); setError("") } catch (err) { setError(err instanceof Error ? err.message : "Could not connect to benchmark API.") } }
  useEffect(() => { void load() }, [])
  useEffect(() => {
    let socket: WebSocket | null = null
    let reconnectTimer: number | undefined
    let stopped = false
    const connect = () => {
      if (stopped) return
      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:"
      socket = new WebSocket(`${protocol}//${window.location.host}/api/v1/dashboard/events`)
      socket.onmessage = () => { void load(selected?.id) }
      socket.onclose = () => { if (!stopped) reconnectTimer = window.setTimeout(connect, 2000) }
    }
    connect()
    return () => { stopped = true; if (reconnectTimer) window.clearTimeout(reconnectTimer); socket?.close() }
  }, [selected?.id])
  const download = async () => { if (!selected) return; const data = await exportExperiment(selected.id); const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: "application/json" })); const anchor = document.createElement("a"); anchor.href = url; anchor.download = `${selected.id}.json`; anchor.click(); URL.revokeObjectURL(url) }
  return <main className="min-h-screen bg-[#f8fafc] text-foreground"><div className="mx-auto grid min-h-screen max-w-360 lg:grid-cols-[232px_1fr]"><aside className="border-b bg-card px-4 py-5 lg:border-r lg:border-b-0"><div className="flex items-center gap-3 px-2"><div className="grid size-9 place-items-center rounded-lg bg-slate-900 text-white"><FlaskConical className="size-5" /></div><div><p className="font-semibold">Agentic Lab</p><p className="text-xs text-muted-foreground">Benchmark workspace</p></div></div><nav className="mt-8 space-y-1"><NavLink to="/dashboard" className={navClass}>Dashboard &amp; Metrics</NavLink><NavLink to="/benchmark-testing" className={navClass}>Benchmark Testing</NavLink><NavLink to="/downloads" className={navClass}>Download Manager</NavLink></nav></aside><div className="min-w-0"><header className="flex flex-wrap items-center justify-between gap-3 border-b bg-card px-5 py-4 sm:px-8"><div><p className="text-xs font-medium uppercase tracking-[0.14em] text-blue-700">Benchmark dashboard</p><h1 className="mt-1 text-xl font-semibold">Incident-reporting experiments</h1></div><div className="flex gap-2"><Button variant="outline" onClick={() => void load(selected?.id)}><RefreshCw /> Refresh</Button><Link to="/benchmark-testing"><Button>New experiment</Button></Link>{selected && <Button variant="outline" onClick={() => void download()}><Download /> JSON</Button>}</div></header><div className="space-y-6 px-5 py-7 sm:px-8">{error && <p className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-700">{error}</p>}<section className="rounded-xl border bg-card p-5 shadow-sm"><div className="flex flex-wrap items-center justify-between gap-3"><div><h2 className="font-semibold">Metrics overview</h2><p className="mt-1 text-sm text-muted-foreground">Pheme metrics from phone voice experiments.</p></div></div>{voiceMetrics ? <div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">{summarizeVoiceMetrics(voiceMetrics.batches).map(metric => <Metric key={`${metric.name}-${metric.unit}`} label={metric.name} value={formatMetric(metric.mean, metric.unit)} />)}</div> : null}<div className="mt-5"><ComparisonCharts experiments={experiments.filter(item => item.dataset === "phone")} /></div></section><section className="rounded-xl border bg-card p-5 shadow-sm"><div className="flex flex-wrap items-center justify-between gap-3"><div><h2 className="font-semibold">All experiments ({experiments.length})</h2><p className="mt-1 text-sm text-muted-foreground">Select an experiment to inspect its time-based metrics and raw data.</p></div></div>{experiments.length === 0 ? <p className="mt-4 text-sm text-muted-foreground">No incident-reporting experiments have been created yet.</p> : <div className="mt-4 overflow-x-auto"><table className="w-full text-left text-sm"><thead className="border-b text-xs text-muted-foreground"><tr><th className="pb-3">ID</th><th className="pb-3">Model</th><th className="pb-3">Dataset</th><th className="pb-3">Status</th><th className="pb-3">Created</th></tr></thead><tbody>{experiments.map(item => <tr key={item.id} onClick={() => navigate(`/experiments/${encodeURIComponent(item.id)}`)} className="cursor-pointer border-b last:border-0 hover:bg-muted/50"><td className="py-3 font-mono text-xs">{item.id}</td><td className="py-3">{item.model_config.name ?? "—"}</td><td className="py-3">{item.dataset || "—"}</td><td className="py-3 capitalize">{item.status}</td><td className="py-3 text-muted-foreground">{formatDate(item.created_at)}</td></tr>)}</tbody></table></div>}</section></div></div></div></main>
}
function summarizeVoiceMetrics(batches: MetricBatch[]) { const values = new Map<string, { name: string; unit: string; values: number[] }>(); batches.forEach(batch => batch.events.forEach(event => { if (typeof event.value !== "number") return; const key = `${event.name}|${event.unit}`; const item = values.get(key) ?? { name: event.name, unit: event.unit, values: [] }; item.values.push(event.value); values.set(key, item) })); return [...values.values()].map(item => ({ name: item.name, unit: item.unit, mean: item.values.reduce((sum, value) => sum + value, 0) / item.values.length })) }
function formatMetric(value: number, unit: string) { return unit === "percent" || unit === "%" ? `${value.toFixed(2)}%` : `${value.toFixed(2)} ${unit}` }
function Metric({ label, value }: { label: string; value: string }) { return <div className="rounded-xl border bg-muted/30 p-4"><p className="text-sm text-muted-foreground">{label}</p><p className="mt-2 truncate text-xl font-semibold capitalize">{value}</p></div> }
