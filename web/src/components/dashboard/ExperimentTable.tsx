import { ArrowRight } from 'lucide-react'

import type { Experiment } from '@/data/dashboardMockData'

const statusClasses = {
  Completed: 'bg-emerald-50 text-emerald-700 ring-emerald-600/15',
  Running: 'bg-blue-50 text-blue-700 ring-blue-600/15',
  Queued: 'bg-muted text-muted-foreground ring-border',
}

export function ExperimentTable({ experiments }: { experiments: Experiment[] }) {
  return (
    <section className="overflow-hidden rounded-xl border bg-card shadow-sm">
      <div className="flex items-center justify-between border-b px-5 py-4"><div><h2 className="font-semibold">Recent experiments</h2><p className="mt-0.5 text-sm text-muted-foreground">Comparable warm-run results</p></div><button className="inline-flex items-center gap-1 text-sm font-medium text-blue-700 hover:text-blue-800">View all <ArrowRight className="size-4" /></button></div>
      <div className="overflow-x-auto"><table className="w-full min-w-180 text-left text-sm"><thead className="bg-muted/50 text-xs uppercase tracking-wide text-muted-foreground"><tr><th className="px-5 py-3 font-medium">Experiment</th><th className="px-5 py-3 font-medium">Configuration</th><th className="px-5 py-3 font-medium">Status</th><th className="px-5 py-3 font-medium">Response</th><th className="px-5 py-3 font-medium">Task success</th><th className="px-5 py-3 font-medium">Completed</th></tr></thead><tbody className="divide-y">{experiments.map((experiment) => <tr key={experiment.id} className="hover:bg-muted/40"><td className="px-5 py-4 font-medium">{experiment.id}</td><td className="px-5 py-4"><p>{experiment.model} · {experiment.quantization}</p><p className="mt-1 text-xs text-muted-foreground">{experiment.dataset} · {experiment.device}</p></td><td className="px-5 py-4"><span className={`inline-flex rounded-full px-2 py-1 text-xs font-medium ring-1 ${statusClasses[experiment.status]}`}>{experiment.status}</span></td><td className="px-5 py-4 font-medium">{experiment.latency}</td><td className="px-5 py-4 font-medium">{experiment.quality}</td><td className="px-5 py-4 whitespace-nowrap text-muted-foreground">{experiment.timestamp}</td></tr>)}</tbody></table></div>
    </section>
  )
}
