import { ArrowUpRight, Minus } from 'lucide-react'

import type { DashboardMetric } from '@/data/dashboardMockData'

const toneClasses = {
  good: 'bg-emerald-50 text-emerald-700 ring-emerald-600/15',
  watch: 'bg-amber-50 text-amber-700 ring-amber-600/15',
  neutral: 'bg-muted text-muted-foreground ring-border',
}

export function MetricCard({ metric }: { metric: DashboardMetric }) {
  const isUnavailable = metric.value === 'Unavailable'

  return (
    <article className="min-w-0 overflow-hidden rounded-xl border bg-card p-5 shadow-sm">
      <p className="text-sm font-medium text-muted-foreground">{metric.label}</p>
      <div className="mt-3 flex min-w-0 flex-col items-start gap-2 sm:flex-row sm:flex-wrap sm:items-end sm:justify-between">
        <p className={isUnavailable ? 'text-xl font-semibold' : 'text-3xl font-semibold tracking-tight'}>{metric.value}</p>
        <span className={`inline-flex min-w-0 max-w-full items-center gap-1 rounded-full px-2 py-1 text-xs font-medium leading-4 wrap-break-word ring-1 ${toneClasses[metric.tone]}`}>
          {metric.tone === 'good' ? <ArrowUpRight className="size-3 shrink-0" /> : <Minus className="size-3 shrink-0" />}
          {metric.target}
        </span>
      </div>
      <p className="mt-3 text-xs text-muted-foreground">{metric.detail}</p>
    </article>
  )
}
