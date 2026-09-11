import { CheckCircle2 } from 'lucide-react'

import type { QualityMetric } from '@/data/dashboardMockData'

export function QualityBreakdown({ metrics }: { metrics: QualityMetric[] }) {
  return (
    <div className="mt-5 space-y-5">
      {metrics.map((metric) => {
        const meetsTarget = metric.higherIsBetter ? metric.value >= metric.target : metric.value <= metric.target
        return <div key={metric.name}>
          <div className="flex items-center justify-between gap-3 text-sm"><span className="font-medium">{metric.name}</span><span className="font-semibold">{metric.value.toFixed(1)}{metric.unit}</span></div>
          <div className="mt-2 h-2 overflow-hidden rounded-full bg-muted"><div className="h-full rounded-full bg-emerald-500" style={{ width: `${metric.value}%` }} /></div>
          <div className="mt-1.5 flex items-center justify-between text-xs text-muted-foreground"><span>Target: {metric.higherIsBetter ? '≥' : '≤'} {metric.target}{metric.unit}</span>{meetsTarget && <span className="inline-flex items-center gap-1 text-emerald-700"><CheckCircle2 className="size-3" /> On target</span>}</div>
        </div>
      })}
    </div>
  )
}
