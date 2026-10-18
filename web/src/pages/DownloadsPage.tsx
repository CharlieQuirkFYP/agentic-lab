import { useState } from "react"
import { Download, FileJson, Trash2 } from "lucide-react"
import { NavLink } from "react-router-dom"

import { Button } from "@/components/ui/button"
import { clearDownloads, downloadFile, getDownloads, removeDownload, type DownloadRecord } from "@/data/downloadStore"

function DownloadsPage() {
  const [downloads, setDownloads] = useState<DownloadRecord[]>(getDownloads)

  const refresh = () => setDownloads(getDownloads())
  const remove = (id: string) => { removeDownload(id); refresh() }
  const clear = () => { clearDownloads(); refresh() }

  return <main className="min-h-screen bg-[#f8fafc] text-foreground"><div className="mx-auto grid min-h-screen max-w-360 lg:grid-cols-[232px_1fr]">
    <aside className="border-b bg-card px-4 py-5 lg:border-r lg:border-b-0"><div className="flex items-center gap-3 px-2"><div className="grid size-9 place-items-center rounded-lg bg-slate-900 text-white"><Download className="size-5" /></div><div><p className="font-semibold tracking-tight">Agentic Lab</p><p className="text-xs text-muted-foreground">Research workspace</p></div></div>
      <nav className="mt-8 flex gap-1 overflow-x-auto lg:block lg:space-y-1"><NavLink to="/" className={navClass}>Dashboard / Results</NavLink><NavLink to="/benchmark-testing" className={navClass}>Benchmark Testing</NavLink><NavLink to="/downloads" className={({ isActive }) => navClass({ isActive })}>Download Manager</NavLink></nav>
    </aside>
    <div className="min-w-0"><header className="flex items-center justify-between border-b bg-card px-5 py-4 sm:px-8"><div><p className="text-xs font-medium uppercase tracking-[0.14em] text-blue-700">Results management</p><h1 className="mt-1 text-xl font-semibold tracking-tight">Download Manager</h1></div>{downloads.length > 0 && <Button variant="outline" onClick={clear}><Trash2 /> Clear history</Button>}</header>
      <div className="px-5 py-7 sm:px-8"><section className="rounded-xl border bg-card p-5 shadow-sm"><div className="flex items-start gap-3"><FileJson className="mt-0.5 size-5 text-blue-600" /><div><h2 className="font-semibold">Previous exports</h2><p className="mt-1 text-sm text-muted-foreground">Exports are stored locally in this browser until the backend storage is connected.</p></div></div>
        {downloads.length === 0 ? <div className="mt-6 rounded-lg border border-dashed px-4 py-10 text-center text-sm text-muted-foreground">No exports yet. Generate a JSON or CSV export from Dashboard / Results.</div> : <div className="mt-6 divide-y">{downloads.map((download) => <div key={download.id} className="flex flex-col gap-3 py-4 sm:flex-row sm:items-center sm:justify-between"><div><p className="font-medium">{download.fileName}</p><p className="mt-1 text-xs text-muted-foreground">{download.format.toUpperCase()} · {new Date(download.createdAt).toLocaleString()}</p></div><div className="flex gap-2"><Button variant="outline" onClick={() => downloadFile(download)}><Download /> Download again</Button><Button variant="ghost" onClick={() => remove(download.id)}><Trash2 /></Button></div></div>)}</div>}
      </section></div>
    </div>
  </div></main>
}

function navClass({ isActive }: { isActive: boolean }) {
  return isActive ? "block rounded-lg bg-slate-900 px-3 py-2 text-sm font-medium text-white" : "block rounded-lg px-3 py-2 text-sm font-medium text-muted-foreground hover:bg-muted"
}

export default DownloadsPage
