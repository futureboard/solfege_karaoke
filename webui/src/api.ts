import { useEffect, useRef, useState } from 'react'
import type { Action, AppState, BrowseResult, Meters, PresetInfo } from './types'

let socket: WebSocket | null = null

/** Send an action over the WebSocket when open, otherwise via POST. */
export function act(action: Action): void {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(action))
    return
  }
  void fetch('/api/action', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(action),
  })
}

export async function getState(): Promise<AppState> {
  const r = await fetch('/api/state')
  return r.json()
}

const presetCache = new Map<string, PresetInfo[]>()

/** Preset list of a slot; cached per slot id + instrument path. */
export async function getPresets(slot: number, path: string): Promise<PresetInfo[]> {
  const key = `${slot}:${path}`
  const hit = presetCache.get(key)
  if (hit) return hit
  const r = await fetch(`/api/slots/${slot}/presets`)
  if (!r.ok) return []
  const list: PresetInfo[] = await r.json()
  presetCache.set(key, list)
  return list
}

export async function browse(path: string | null, kind: 'instrument' | 'song'): Promise<BrowseResult> {
  const q = new URLSearchParams({ kind })
  if (path) q.set('path', path)
  const r = await fetch(`/api/browse?${q}`)
  if (!r.ok) throw new Error(await r.text())
  return r.json()
}

/**
 * Live connection: meters stream over the WebSocket (~25 Hz); whenever the
 * server's state version changes the full state is refetched.
 */
export function useBackend() {
  const [state, setState] = useState<AppState | null>(null)
  const [meters, setMeters] = useState<Meters | null>(null)
  const [connected, setConnected] = useState(false)
  const version = useRef(-1)

  useEffect(() => {
    let closed = false
    let retry: number | undefined

    const refresh = () => getState().then((s) => !closed && setState(s)).catch(() => {})

    const connect = () => {
      const proto = location.protocol === 'https:' ? 'wss' : 'ws'
      const ws = new WebSocket(`${proto}://${location.host}/ws`)
      socket = ws
      ws.onopen = () => {
        setConnected(true)
        refresh()
      }
      ws.onmessage = (ev) => {
        const m = JSON.parse(ev.data) as Meters
        if (m.type !== 'meters') return
        if (m.version !== version.current) {
          version.current = m.version
          refresh()
        }
        setMeters(m)
      }
      ws.onclose = () => {
        setConnected(false)
        if (socket === ws) socket = null
        if (!closed) retry = window.setTimeout(connect, 1000)
      }
    }
    connect()
    return () => {
      closed = true
      window.clearTimeout(retry)
      socket?.close()
    }
  }, [])

  return { state, meters, connected }
}

const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
export const noteName = (n: number) => `${NAMES[n % 12]}${Math.floor(n / 12) - 1}`
export const toDb = (x: number) => (x <= 1e-6 ? -120 : 20 * Math.log10(x))
export const fmtTime = (t: number) => {
  const m = Math.floor(Math.max(0, t) / 60)
  const s = Math.max(0, t) % 60
  return `${m}:${s.toFixed(1).padStart(4, '0')}`
}
export const outputName = (bus: number) => (bus === 0 ? 'Main' : `${bus * 2 + 1}/${bus * 2 + 2}`)
