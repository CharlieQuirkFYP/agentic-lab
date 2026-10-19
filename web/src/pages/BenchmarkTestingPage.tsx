import { useMemo, useState } from 'react'
import { CheckCircle2, FileJson, Play, RotateCcw, Upload, XCircle } from 'lucide-react'
import { NavLink } from 'react-router-dom'

import { Button } from '@/components/ui/button'

type RunStatus = 'Ready' | 'Queued' | 'Running' | 'Completed' | 'Failed'
const sample = JSON.stringify({ name: 'incident-reporting-smoke', dataset: 'incident-v03', cases: [
  { id: 'case-001', transcript: 'Smoke reported near the north stairwell.' },
  { id: 'case-002', transcript: 'A vehicle collided with a barrier near the west entrance.' },
]}, null, 2)
const statusStyles: Record<RunStatus, string> = { Ready: 'bg-muted text-muted-foreground', Queued: 'bg-amber-50 text-amber-700', Running: 'bg-blue-50 text-blue-700', Completed: 'bg-emerald-50 text-emerald-700', Failed: 'bg-red-50 text-red-700' }

function BenchmarkTestingPage() {
  const [jsonText, setJsonText] = useState(sample)
  const [status, setStatus] = useState<RunStatus>('Ready')
  const [message, setMessage] = useState('Paste JSON and run a local validation.')
  const [lastRun, setLastRun] = useState<string | null>(null)
  const parsed = useMemo(() => {
    try {
      const value = JSON.parse(jsonText) as { cases?: unknown[] }
      return Array.isArray(value.cases) && value.cases.length > 0
        ? { valid: true, detail: String(value.cases.length) + ' test cases ready to run.' }
        : { valid: false, detail: 'JSON must contain a non-empty cases array.' }
    } catch { return { valid: false, detail: 'The JSON is not valid.' } }
  }, [jsonText])

  const runTest = () => {
    if (!parsed.valid) { setStatus('Failed'); setMessage(parsed.detail); return }
    setStatus('Queued'); setMessage('Test queued locally. The backend runner will replace this simulation.')
    window.setTimeout(() => { setStatus('Running'); setMessage('Running transcript cases against the selected profile…') }, 350)
    window.setTimeout(() => { setStatus('Completed'); setLastRun(new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })); setMessage('Validation completed. Results are ready for the dashboard.') }, 1200)
  }
  const reset = () => { setJsonText(sample); setStatus('Ready'); setLastRun(null); setMessage('Paste JSON and run a local validation.') }

  return <main className="min-h-screen bg-[#f8fafc] text-foreground"><div className="mx-auto grid min-h-screen max-w-360 lg:grid-cols-[232px_1fr]">
    <aside className="border-b bg-card px-4 py-5 lg:border-r lg:border-b-0"><div className="flex items-center gap-3 px-2"><div className="grid size-9 place-items-center rounded-lg bg-slate-900 text-white"><FileJson className="size-5" /></div><div><p className="font-semibold tracking-tight">Agentic Lab</p><p className="text-xs text-muted-foreground">Research workspace</p></div></div>
      <nav className="mt-8 flex gap-1 overflow-x-auto lg:block lg:space-y-1"><NavLink to="/dashboard" className={({ isActive }) => isActive ? 'block rounded-lg bg-slate-900 px-3 py-2 text-sm font-medium text-white' : 'block rounded-lg px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-muted'}>Dashboard / Results</NavLink><NavLink to="/benchmark-testing" className={({ isActive }) => isActive ? 'block rounded-lg bg-slate-900 px-3 py-2 text-sm font-medium text-white' : 'block rounded-lg px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-muted'}>Benchmark Testing</NavLink><NavLink to="/downloads" className={({ isActive }) => isActive ? "block rounded-lg bg-slate-900 px-3 py-2 text-sm font-medium text-white" : "block rounded-lg px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-muted"}>Download Manager</NavLink></nav>
      <div className="mt-8 hidden rounded-lg border bg-muted/50 p-3 lg:block"><p className="text-xs font-medium">Frontend-only mode</p><p className="mt-1 text-xs leading-5 text-muted-foreground">Local validation until the Go benchmark API is connected.</p></div>
    </aside>
    <div className="min-w-0"><header className="flex flex-col gap-4 border-b bg-card px-5 py-4 sm:flex-row sm:items-center sm:justify-between sm:px-8"><div><p className="text-xs font-medium uppercase tracking-[0.14em] text-blue-700">Benchmark testing</p><h1 className="mt-1 text-xl font-semibold tracking-tight">Run a JSON test set</h1></div><div className="flex gap-2"><Button variant="outline" onClick={reset}><RotateCcw /> Reset</Button><Button onClick={runTest}><Play /> Run test</Button></div></header>
      <div className="grid gap-6 px-5 py-7 sm:px-8 xl:grid-cols-[1.35fr_0.65fr]"><section className="rounded-xl border bg-card p-5 shadow-sm"><div className="flex items-start justify-between gap-4"><div><div className="flex items-center gap-2"><FileJson className="size-4 text-blue-600" /><h2 className="font-semibold">JSON Testing Data Module</h2></div><p className="mt-1 text-sm text-muted-foreground">Define transcript cases for the benchmark runner.</p></div><label className="inline-flex cursor-pointer items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium hover:bg-muted"><Upload className="size-3.5" /> Import<input className="hidden" type="file" accept=".json,application/json" onChange={(event) => { const file = event.target.files?.[0]; if (file) void file.text().then(setJsonText) }} /></label></div><textarea value={jsonText} onChange={(event) => setJsonText(event.target.value)} spellCheck={false} className="mt-5 min-h-105 w-full rounded-lg border bg-slate-950 p-4 font-mono text-sm leading-6 text-slate-100 outline-none focus:ring-2 focus:ring-blue-500" aria-label="Benchmark JSON data" /><div className={parsed.valid ? 'mt-3 flex items-center gap-2 text-sm text-emerald-700' : 'mt-3 flex items-center gap-2 text-sm text-red-700'}>{parsed.valid ? <CheckCircle2 className="size-4" /> : <XCircle className="size-4" />}{parsed.detail}</div></section>
        <div className="space-y-6"><section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Testing Status Module</h2><div className="mt-5 flex items-center justify-between gap-3"><span className={'rounded-full px-2.5 py-1 text-xs font-semibold ' + statusStyles[status]}>{status}</span>{lastRun && <span className="text-xs text-muted-foreground">Last run {lastRun}</span>}</div><p className="mt-4 text-sm leading-6 text-muted-foreground">{message}</p><div className="mt-5 h-2 overflow-hidden rounded-full bg-muted"><div className={'h-full rounded-full transition-all ' + progressClass(status)} /></div></section><section className="rounded-xl border bg-card p-5 shadow-sm"><h2 className="font-semibold">Test configuration</h2><div className="mt-4 space-y-3 text-sm"><label className="grid gap-1 text-xs font-medium text-muted-foreground">Reference profile<select className="h-9 rounded-lg border bg-card px-2 text-sm font-normal text-foreground" defaultValue="reference"><option value="reference">Reference CPU</option><option value="mobile">Mobile baseline</option><option value="laptop">Laptop baseline</option></select></label><label className="grid gap-1 text-xs font-medium text-muted-foreground">Run mode<select className="h-9 rounded-lg border bg-card px-2 text-sm font-normal text-foreground" defaultValue="warm"><option value="warm">Warm run</option><option value="cold">Cold start</option></select></label></div></section><section className="rounded-xl border border-dashed bg-card p-5"><p className="text-sm font-medium">Backend connection</p><p className="mt-1 text-sm leading-6 text-muted-foreground">The UI is ready for the Go API. Current runs use a local mock transition.</p></section></div></div>
    </div>
  </div></main>
}

function progressClass(status: RunStatus) { return status === 'Completed' ? 'w-full bg-emerald-500' : status === 'Failed' ? 'w-full bg-red-500' : status === 'Running' ? 'w-2/3 bg-blue-500' : status === 'Queued' ? 'w-1/3 bg-amber-500' : 'w-0 bg-transparent' }
export default BenchmarkTestingPage
