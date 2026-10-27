import { useState } from "react"
import { Button } from "@/components/ui/button"

type MetricFilterProps = {
  metricNames: string[]
  activeNames: Set<string>
  allSelected: boolean
  toggle: (name: string) => void
  toggleAll: () => void
}

export default function MetricFilter({ metricNames, activeNames, allSelected, toggle, toggleAll }: MetricFilterProps) {
  const [open, setOpen] = useState(false)
  return <div className="relative">
    <div className="flex items-center gap-2">
      <span className="hidden text-xs text-muted-foreground sm:inline">{activeNames.size} of {metricNames.length} shown</span>
      <Button variant="outline" onClick={() => setOpen(value => !value)}>Filter metrics <span className="text-muted-foreground">⌄</span></Button>
    </div>
    {open && <div className="absolute right-0 z-20 mt-2 w-80 rounded-xl border bg-card p-3 shadow-lg">
      <div className="flex items-center justify-between border-b px-1 pb-3"><span className="text-sm font-medium">Choose metrics</span><button type="button" className="text-xs font-medium text-blue-700" onClick={toggleAll}>{allSelected ? "Unselect all" : "Select all"}</button></div>
      <div className="mt-2 max-h-72 overflow-y-auto">{metricNames.map(name => <label key={name} className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-2 text-sm hover:bg-muted"><input type="checkbox" checked={activeNames.has(name)} onChange={() => toggle(name)} /><span className="truncate">{name}</span></label>)}</div>
    </div>}
  </div>
}
