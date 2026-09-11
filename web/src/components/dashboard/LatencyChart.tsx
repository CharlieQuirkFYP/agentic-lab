import type { TrendPoint } from '@/data/dashboardMockData'

export function LatencyChart({ points }: { points: TrendPoint[] }) {
  const width = 620
  const height = 210
  const padding = { top: 20, right: 14, bottom: 30, left: 38 }
  const chartWidth = width - padding.left - padding.right
  const chartHeight = height - padding.top - padding.bottom
  const max = 12
  const pointPosition = (point: TrendPoint, index: number) => ({
    x: padding.left + (index / (points.length - 1)) * chartWidth,
    y: padding.top + chartHeight - (point.latency / max) * chartHeight,
  })
  const positions = points.map(pointPosition)
  const line = positions.map(({ x, y }, index) => `${index === 0 ? 'M' : 'L'} ${x} ${y}`).join(' ')
  const area = `${line} L ${positions.at(-1)?.x} ${padding.top + chartHeight} L ${positions[0]?.x} ${padding.top + chartHeight} Z`
  const targetY = padding.top + chartHeight - (10 / max) * chartHeight

  return (
    <div className="mt-5 h-56">
      <svg className="size-full overflow-visible" viewBox={`0 0 ${width} ${height}`} role="img" aria-label="Median end-to-end response time over seven days">
        {[0, 4, 8, 12].map((value) => {
          const y = padding.top + chartHeight - (value / max) * chartHeight
          return <g key={value}><line x1={padding.left} x2={width - padding.right} y1={y} y2={y} className="stroke-border" strokeDasharray="3 4" /><text x="0" y={y + 4} className="fill-muted-foreground text-[10px]">{value}s</text></g>
        })}
        <line x1={padding.left} x2={width - padding.right} y1={targetY} y2={targetY} className="stroke-amber-500" strokeDasharray="5 5" />
        <text x={width - padding.right} y={targetY - 6} textAnchor="end" className="fill-amber-600 text-[10px]">10s target</text>
        <path d={area} className="fill-blue-500/10" />
        <path d={line} fill="none" className="stroke-blue-600" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
        {positions.map(({ x, y }, index) => <g key={points[index].label}><circle cx={x} cy={y} r="3.5" className="fill-card stroke-blue-600" strokeWidth="2" /><text x={x} y={height - 5} textAnchor="middle" className="fill-muted-foreground text-[10px]">{points[index].label}</text></g>)}
      </svg>
    </div>
  )
}
