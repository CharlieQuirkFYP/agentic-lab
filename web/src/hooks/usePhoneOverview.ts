import { useEffect, useRef, useState } from 'react'
import { getPhoneOverview } from '../lib/phoneBenchmarkApi.ts'
import { createCoalescedRefresh, type PhoneFilters, type PhoneOverview } from '../lib/phoneBenchmarkTypes.ts'

export function usePhoneOverview(filters: PhoneFilters) {
  const [result, setResult] = useState<{ key: string; overview: PhoneOverview; receivedAt: number } | null>(null)
  const [failure, setFailure] = useState<{ key: string; message: string } | null>(null)
  const [connection, setConnection] = useState<'connecting' | 'live' | 'reconnecting'>('connecting')
  const [now, setNow] = useState(() => Date.now())
  const refreshRef = useRef<() => void>(() => {})
  const key = JSON.stringify(filters)
  useEffect(() => {
    const selectedFilters = JSON.parse(key) as PhoneFilters
    const refresh = createCoalescedRefresh(signal => getPhoneOverview(selectedFilters, signal), result => {
      setResult({ key, overview: result, receivedAt: Date.now() }); setFailure(null)
    }, reason => setFailure({ key, message: reason instanceof Error ? reason.message : 'Could not load phone metrics.' }))
    refreshRef.current = () => { void refresh.refresh() }
    let socket: WebSocket | null = null
    let reconnectTimer: number | undefined
    let debounce: number | undefined
    let stopped = false
    const schedule = () => {
      if (debounce !== undefined) return
      debounce = window.setTimeout(() => { debounce = undefined; void refresh.refresh() }, 250)
    }
    const connect = () => {
      if (stopped) return
      const base = new URL(import.meta.env.VITE_API_BASE_URL ?? '/api/v1', window.location.href)
      base.protocol = base.protocol === 'https:' ? 'wss:' : 'ws:'
      base.pathname = `${base.pathname.replace(/\/$/, '')}/dashboard/events`
      socket = new WebSocket(base)
      socket.onopen = () => { setConnection('live'); schedule() }
      socket.onmessage = event => {
        try { const data = JSON.parse(event.data) as { type?: string }; if (data.type?.endsWith('.updated')) schedule() } catch { /* Ignore malformed notifications. */ }
      }
      socket.onclose = () => { if (!stopped) { setConnection('reconnecting'); reconnectTimer = window.setTimeout(connect, 2000) } }
    }
    void refresh.refresh()
    connect()
    const fallback = window.setInterval(() => { setNow(Date.now()); schedule() }, 30000)
    return () => {
      stopped = true; refresh.dispose(); socket?.close(); window.clearInterval(fallback)
      window.clearTimeout(reconnectTimer); window.clearTimeout(debounce)
    }
  }, [key])
  return { overview: result?.key === key ? result.overview : null, error: failure?.key === key ? failure.message : '', connection, receivedAt: result?.key === key ? result.receivedAt : null, now, refresh: () => refreshRef.current() }
}
