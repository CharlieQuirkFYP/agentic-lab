const STORAGE_KEY = "agentic-lab-downloads"

export type DownloadFormat = "json" | "csv"
export type DownloadRecord = {
  id: string
  fileName: string
  format: DownloadFormat
  createdAt: string
  content: string
}

export function getDownloads(): DownloadRecord[] {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]") as DownloadRecord[]
  } catch {
    return []
  }
}

export function saveDownload(record: Omit<DownloadRecord, "id" | "createdAt">): DownloadRecord {
  const saved: DownloadRecord = { ...record, id: crypto.randomUUID(), createdAt: new Date().toISOString() }
  localStorage.setItem(STORAGE_KEY, JSON.stringify([saved, ...getDownloads()].slice(0, 20)))
  return saved
}

export function removeDownload(id: string): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(getDownloads().filter((download) => download.id !== id)))
}

export function clearDownloads(): void {
  localStorage.removeItem(STORAGE_KEY)
}

export function downloadFile(download: Pick<DownloadRecord, "fileName" | "format" | "content">): void {
  const mimeType = download.format === "csv" ? "text/csv" : "application/json"
  const blob = new Blob([download.content], { type: mimeType })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement("a")
  anchor.href = url
  anchor.download = download.fileName
  anchor.click()
  URL.revokeObjectURL(url)
}
