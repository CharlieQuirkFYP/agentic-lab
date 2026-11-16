import { useState } from 'react'
import { Download } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { getPhoneSnapshot } from '@/lib/phoneBenchmarkApi'
import type { PhoneFilters } from '@/lib/phoneBenchmarkTypes'
import { createBenchmarkReport, reportToCSV, reportToJSON, reportToMarkdown } from '../../../../shared/phone-benchmarks/index.ts'

export default function PhoneExport({ filters, asOf }: { filters: PhoneFilters; asOf?: string }) {
  const [format, setFormat] = useState<'json' | 'csv' | 'md'>('json')
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const [error, setError] = useState('')
  const download = async () => {
    if (!asOf || busy) return
    setBusy(true); setError('')
    try {
      const snapshot = await getPhoneSnapshot(filters, asOf)
      const report = createBenchmarkReport(snapshot.records, { generated_at_ms: Date.now(), as_of: snapshot.as_of, filters, legacy_records: snapshot.legacy, input_already_filtered: true, selected_record_keys: snapshot.selected_record_keys })
      const content = format === 'json' ? reportToJSON(report) : format === 'csv' ? reportToCSV(report) : reportToMarkdown(report)
      const url = URL.createObjectURL(new Blob([content], { type: format === 'json' ? 'application/json' : format === 'csv' ? 'text/csv;charset=utf-8' : 'text/markdown;charset=utf-8' }))
      const anchor = document.createElement('a')
      anchor.href = url; anchor.download = `phone-benchmarks-${new Date().toISOString().replace(/[:.]/g, '-')}.${format}`
      anchor.click(); URL.revokeObjectURL(url)
      setMessage(`Exported complete selected snapshot ${asOf}.`)
    } catch (reason) { setError(reason instanceof Error ? reason.message : 'Could not export phone observations.') }
    finally { setBusy(false) }
  }
  return <div><div className="flex gap-2"><select aria-label="Report format" value={format} onChange={event => setFormat(event.target.value as typeof format)} className="rounded-md border bg-card px-2 text-xs"><option value="json">JSON</option><option value="csv">CSV</option><option value="md">Markdown</option></select><Button variant="outline" disabled={!asOf || busy} onClick={() => void download()}><Download className="size-4" />{busy ? 'Preparing snapshot…' : 'Export phone report'}</Button></div>{message && <p className="mt-2 max-w-sm break-all text-xs text-muted-foreground" role="status">{message}</p>}{error && <p className="mt-2 max-w-sm text-xs text-red-700" role="alert">{error}</p>}</div>
}
