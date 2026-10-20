import { useEffect, useMemo, useState } from "react"

export const METRIC_FILTER_KEY = "agentic-lab.metric-filter"

function readSaved(): string[] | null {
  try {
    const raw = localStorage.getItem(METRIC_FILTER_KEY)
    if (raw === null) return null
    const value = JSON.parse(raw)
    return Array.isArray(value) ? value as string[] : null
  } catch {
    return null
  }
}

export function useMetricFilter(metricNames: string[]) {
  const [selected, setSelected] = useState<string[] | null>(readSaved)
  const activeNames = selected === null ? metricNames : selected
  const allSelected = metricNames.length > 0 && metricNames.every(name => activeNames.includes(name))

  useEffect(() => {
    if (selected === null) localStorage.removeItem(METRIC_FILTER_KEY)
    else localStorage.setItem(METRIC_FILTER_KEY, JSON.stringify(selected))
  }, [selected])

  return {
    activeNames: useMemo(() => new Set(activeNames), [activeNames.join("\u0000")]),
    allSelected,
    toggle: (name: string) => setSelected(current => {
      const active = current === null ? metricNames : current
      return active.includes(name) ? active.filter(item => item !== name) : [...active, name]
    }),
    toggleAll: () => setSelected(allSelected ? [] : metricNames),
  }
}
