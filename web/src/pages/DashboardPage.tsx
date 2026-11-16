import { useEffect, useRef, useState } from "react"
import { Link, NavLink, useNavigate, useSearchParams } from "react-router-dom"
import { Download, FlaskConical, RefreshCw } from "lucide-react"
import { Button } from "@/components/ui/button"
import { exportExperiment, listExperiments, type Experiment } from "@/lib/benchmarkApi"
import { usePhoneOverview } from "@/hooks/usePhoneOverview"
import { CurrentPhoneRun, PhoneFilterBar, PhoneProgress, PhoneSeries } from "@/components/dashboard/PhoneMetrics"
import { phoneDate } from "@/lib/phonePresentation"
import PhoneHistory from "@/components/dashboard/PhoneHistory"
import PhoneExport from "@/components/dashboard/PhoneExport"
import PhoneComparisonCharts from "@/components/dashboard/PhoneComparisonCharts"
import PhoneSnapshotAnalysis from "@/components/dashboard/PhoneSnapshotAnalysis"
import { parsePhoneFilters, type PhoneFilters } from "@/lib/phoneBenchmarkTypes"

const navClass = ({ isActive }: { isActive: boolean }) => isActive ? "block rounded-lg bg-slate-900 px-3 py-2 text-sm font-medium text-white" : "block rounded-lg px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-muted"
const formatDate = (value: string) => new Date(value).toLocaleString()

export default function DashboardPage() {
  const [experiments, setExperiments] = useState<Experiment[]>([])
  const [selected, setSelected] = useState<Experiment | null>(null)
  const [error, setError] = useState("")
  const [params, setParams] = useSearchParams()
  const paramsRef = useRef(params)
  useEffect(() => { paramsRef.current = params }, [params])
  const filters = parsePhoneFilters(params)
  const phone = usePhoneOverview(filters)
  const updateFilter = (key: keyof PhoneFilters, value: string) => {
    const next = new URLSearchParams(paramsRef.current)
    if (value) next.set(key, value); else next.delete(key)
    paramsRef.current = next
    setParams(next, { replace: true })
  }
  const updateView = (view: string) => { const next = new URLSearchParams(params); next.set('view', view); setParams(next, { replace: true }) }
  const selectedIdRef = useRef<string | undefined>(undefined)
  const navigate = useNavigate()
  const load = async (id = selectedIdRef.current) => {
    try {
      const page = await listExperiments()
      setExperiments(page.experiments)
      const next = page.experiments.find(item => item.id === id) ?? page.experiments[0] ?? null
      selectedIdRef.current = next?.id
      setSelected(next)
      setError("")
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not connect to benchmark API.")
    }
  }
  useEffect(() => { void load() }, [])
  const download = async () => { if (!selected) return; const data = await exportExperiment(selected.id); const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: "application/json" })); const anchor = document.createElement("a"); anchor.href = url; anchor.download = `${selected.id}.json`; anchor.click(); URL.revokeObjectURL(url) }
  return <main className="min-h-screen bg-[#f8fafc] text-foreground">
    <div className="mx-auto grid min-h-screen max-w-360 lg:grid-cols-[232px_1fr]">
      <aside className="border-b bg-card px-4 py-5 lg:border-r lg:border-b-0">
        <div className="flex items-center gap-3 px-2">
          <div className="grid size-9 place-items-center rounded-lg bg-slate-900 text-white"><FlaskConical className="size-5" /></div>
          <div><p className="font-semibold">Agentic Lab</p><p className="text-xs text-muted-foreground">Benchmark workspace</p></div>
        </div>
        <nav className="mt-8 space-y-1">
          <NavLink to="/dashboard" className={navClass}>Dashboard &amp; Metrics</NavLink>
          <NavLink to="/benchmark-testing" className={navClass}>Benchmark Testing</NavLink>
          <NavLink to="/downloads" className={navClass}>Download Manager</NavLink>
        </nav>
      </aside>
      <div className="min-w-0">
        <header className="flex flex-wrap items-center justify-between gap-3 border-b bg-card px-5 py-4 sm:px-8">
          <div><p className="text-xs font-medium uppercase tracking-[0.14em] text-blue-700">Benchmark dashboard</p><h1 className="mt-1 text-xl font-semibold">Incident-reporting experiments</h1></div>
          <div className="flex gap-2">
            <Button variant="outline" onClick={() => { void load(selected?.id); phone.refresh() }}><RefreshCw /> Refresh</Button>
            <Link to="/benchmark-testing"><Button>New experiment</Button></Link>
            {selected && <Button variant="outline" onClick={() => void download()}><Download /> JSON</Button>}
          </div>
        </header>
        <div className="space-y-6 px-5 py-7 sm:px-8">
          {error && <p role="alert" className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-700">{error}</p>}
          {phone.error && <p role="alert" className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-700">{phone.error} {phone.overview ? 'Showing the last received snapshot.' : ''}</p>}
          <CurrentPhoneRun overview={phone.overview} search={params.toString()} />
          <PhoneFilterBar filters={filters} update={updateFilter} />
          <section className="rounded-xl border bg-card p-5 shadow-sm">
            <div className="flex flex-wrap items-start justify-between gap-4"><div><h2 className="font-semibold">Metrics overview</h2><p className="mt-1 text-sm text-muted-foreground">All selected device-reported phone observations. Inference stays on the phone.</p></div><PhoneExport filters={filters} asOf={phone.overview?.as_of} /></div>
            {phone.overview ? <><div className="mt-4 rounded-lg bg-muted/40 p-3 text-xs text-muted-foreground"><p>Collection coverage: {phoneDate(phone.overview.coverage.first_timestamp_ms)} – {phoneDate(phone.overview.coverage.last_timestamp_ms)}</p><p className="mt-1">Last server receipt: {phoneDate(phone.overview.coverage.last_received_at_ms)} · Updates {phone.error || (phone.receivedAt !== null && phone.now - phone.receivedAt > 60000) ? 'stale' : phone.connection} · {phone.overview.coverage.event_count.toLocaleString()} total received events</p><details className="mt-1"><summary className="cursor-pointer">Snapshot identity</summary><p className="mt-1 break-all font-mono">{phone.overview.as_of}</p></details></div><PhoneProgress overview={phone.overview} /><PhoneComparisonCharts series={phone.overview.series} /><PhoneSeries series={phone.overview.series} /></> : <p className="mt-4 text-sm text-muted-foreground">{phone.error ? 'Phone overview is unavailable.' : 'Loading the full selected dataset…'}</p>}
          </section>
          {phone.overview && <PhoneSnapshotAnalysis filters={filters} asOf={phone.overview.as_of} />}
          {phone.overview && <PhoneHistory overview={phone.overview} filters={filters} search={params.toString()} view={params.get('view') ?? (phone.overview.counts.conversations.total ? 'conversations' : 'runs')} setView={updateView} />}
          <section className="rounded-xl border bg-card p-5 shadow-sm">
            <h2 className="font-semibold">Experiment browser ({experiments.length} loaded)</h2>
            <p className="mt-1 text-sm text-muted-foreground">Select an experiment to inspect its time-based metrics and raw data. Device-reported attribution is not independently verified.</p>
            {experiments.length === 0 ? <p className="mt-4 text-sm text-muted-foreground">No incident-reporting experiments have been created yet.</p> :
              <div className="mt-4 overflow-x-auto">
                <table className="w-full text-left text-sm">
                  <thead className="border-b text-xs text-muted-foreground">
                    <tr><th className="pb-3">ID</th><th className="pb-3">Model</th><th className="pb-3">Dataset</th><th className="pb-3">Source</th><th className="pb-3">Status</th><th className="pb-3">Created</th></tr>
                  </thead>
                  <tbody>{experiments.map(item => <tr key={item.id}
                    onClick={() => navigate(`/experiments/${encodeURIComponent(item.id)}`)}
                    className="cursor-pointer border-b last:border-0 hover:bg-muted/50">
                    <td className="py-3 font-mono text-xs">{item.id}</td>
                    <td className="py-3">{item.model_config.name ?? "—"}</td>
                    <td className="py-3">{item.dataset || "—"}</td>
                    <td className="py-3">{item.result_source}</td>
                    <td className="py-3 capitalize">{item.status}</td>
                    <td className="py-3 text-muted-foreground">{formatDate(item.created_at)}</td>
                  </tr>)}</tbody>
                </table>
              </div>}
          </section>
        </div>
      </div>
    </div>
  </main>
}
