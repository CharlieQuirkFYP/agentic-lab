import { BarChart3 } from 'lucide-react'
import { Link } from 'react-router-dom'

export default function BenchmarkPage() {
  return (
    <main className="mx-auto max-w-7xl space-y-6 px-5 py-8 sm:px-8">
      <div><p className="text-xs font-medium uppercase tracking-widest text-muted-foreground">Research workspace</p><h1 className="mt-2 text-2xl font-semibold">Benchmark dashboard</h1></div>
      <section className="max-w-3xl rounded-xl border bg-card p-6 sm:p-8">
        <BarChart3 className="mb-4 size-6 text-muted-foreground" aria-hidden="true" />
        <h2 className="text-lg font-medium">Measurements are not available yet</h2>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">The dashboard remains part of Agentic Lab: experiment creation, progress, results, model/device comparisons, and export. It is not implemented in this voice-console increment.</p>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">Existing Go experiments use a mocked runner. No power, energy, latency, quality, or endurance figures are presented here as measured results. Voice-turn timings are not a substitute for a benchmark.</p>
        <Link to="/voice" className="mt-6 inline-block text-sm font-medium underline underline-offset-4">Return to the voice console</Link>
      </section>
    </main>
  )
}
