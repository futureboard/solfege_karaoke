import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { toDb } from '../api'

/** Calls `fn` at most every `ms`, always delivering the last value. */
export function useThrottle<T>(fn: (v: T) => void, ms = 30) {
  const last = useRef(0)
  const pending = useRef<T | null>(null)
  const timer = useRef<number | undefined>(undefined)
  const fnRef = useRef(fn)
  fnRef.current = fn
  useEffect(() => () => window.clearTimeout(timer.current), [])
  return (v: T) => {
    const now = performance.now()
    pending.current = v
    if (now - last.current >= ms) {
      last.current = now
      fnRef.current(v)
      pending.current = null
    } else if (timer.current === undefined) {
      timer.current = window.setTimeout(() => {
        timer.current = undefined
        last.current = performance.now()
        if (pending.current !== null) fnRef.current(pending.current)
        pending.current = null
      }, ms - (now - last.current))
    }
  }
}

/**
 * Value that follows the server but holds the local value while the user
 * is interacting (and briefly after), so echoes do not make it jump.
 */
function useLocalValue(value: number) {
  const [local, setLocal] = useState(value)
  const touched = useRef(0)
  useEffect(() => {
    if (performance.now() - touched.current > 400) setLocal(value)
  }, [value])
  const set = (v: number) => {
    touched.current = performance.now()
    setLocal(v)
  }
  return [local, set] as const
}

// ------------------------------------------------------------------ meter

const dbToFrac = (db: number) => Math.min(1, Math.max(0, (db + 60) / 66))

export function Meter({ level, vertical = true, size = 120 }: { level: [number, number]; vertical?: boolean; size?: number }) {
  const bars = level.map((v) => dbToFrac(toDb(v)))
  return (
    <div className={vertical ? 'meter vertical' : 'meter horizontal'} style={vertical ? { height: size } : { width: size }}>
      {bars.map((f, i) => (
        <div key={i} className="meter-track">
          <div className="meter-fill" style={vertical ? { height: `${f * 100}%` } : { width: `${f * 100}%` }} />
        </div>
      ))}
    </div>
  )
}

export function MiniMeter({ level }: { level: number }) {
  const f = dbToFrac(toDb(level))
  return (
    <div className="mini-meter">
      <div className="meter-fill" style={{ width: `${f * 100}%` }} />
    </div>
  )
}

// ------------------------------------------------------------------- knob

interface KnobProps {
  label: string
  value: number
  min: number
  max: number
  onChange: (v: number) => void
  format?: (v: number) => string
  log?: boolean
  defaultValue?: number
  size?: number
}

/** Drag up/down (Shift = fine), wheel to nudge, double-click to reset. */
export function Knob({ label, value, min, max, onChange, format, log, defaultValue, size = 34 }: KnobProps) {
  const [local, setLocal] = useLocalValue(value)
  const send = useThrottle(onChange)
  const toFrac = (v: number) =>
    log ? Math.log(v / min) / Math.log(max / min) : (v - min) / (max - min)
  const fromFrac = (f: number) => {
    const c = Math.min(1, Math.max(0, f))
    return log ? min * Math.pow(max / min, c) : min + c * (max - min)
  }
  const apply = (v: number) => {
    const c = Math.min(max, Math.max(min, v))
    setLocal(c)
    send(c)
  }
  const drag = useRef<{ y: number; f: number } | null>(null)
  const onDown = (e: ReactPointerEvent) => {
    ;(e.target as Element).setPointerCapture(e.pointerId)
    drag.current = { y: e.clientY, f: toFrac(local) }
  }
  const onMove = (e: ReactPointerEvent) => {
    if (!drag.current) return
    const scale = e.shiftKey ? 600 : 150
    apply(fromFrac(drag.current.f + (drag.current.y - e.clientY) / scale))
  }
  const frac = Math.min(1, Math.max(0, toFrac(local)))
  const angle = -135 + frac * 270
  const r = size / 2 - 3
  const arc = (a: number) => {
    const rad = ((a - 90) * Math.PI) / 180
    return [size / 2 + r * Math.cos(rad), size / 2 + r * Math.sin(rad)]
  }
  const [sx, sy] = arc(-135)
  const [ex, ey] = arc(angle)
  const large = angle + 135 > 180 ? 1 : 0
  return (
    <div className="knob" title={`${label}: ${format ? format(local) : local.toFixed(2)}`}>
      <svg
        width={size}
        height={size}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={() => (drag.current = null)}
        onDoubleClick={() => defaultValue !== undefined && apply(defaultValue)}
        onWheel={(e) => apply(fromFrac(frac - Math.sign(e.deltaY) * 0.02))}
      >
        <circle cx={size / 2} cy={size / 2} r={r} className="knob-bg" />
        {frac > 0.001 && <path d={`M ${sx} ${sy} A ${r} ${r} 0 ${large} 1 ${ex} ${ey}`} className="knob-arc" />}
        <line x1={size / 2} y1={size / 2} x2={ex} y2={ey} className="knob-needle" />
      </svg>
      <span className="knob-label">{label}</span>
      <span className="knob-value">{format ? format(local) : local.toFixed(1)}</span>
    </div>
  )
}

// ------------------------------------------------------------------ fader

interface FaderProps {
  value: number
  min: number
  max: number
  step?: number
  onChange: (v: number) => void
  height?: number
  defaultValue?: number
}

export function Fader({ value, min, max, step = 0.1, onChange, height = 140, defaultValue }: FaderProps) {
  const [local, setLocal] = useLocalValue(value)
  const send = useThrottle(onChange)
  const set = (v: number) => {
    setLocal(v)
    send(v)
  }
  return (
    <input
      className="fader"
      type="range"
      min={min}
      max={max}
      step={step}
      value={local}
      style={{ height }}
      onChange={(e) => set(Number(e.target.value))}
      onDoubleClick={() => defaultValue !== undefined && set(defaultValue)}
    />
  )
}

// ------------------------------------------------------------------ slider

interface SliderProps {
  label: string
  value: number
  min: number
  max: number
  step?: number
  onChange: (v: number) => void
  format?: (v: number) => string
  defaultValue?: number
}

export function Slider({ label, value, min, max, step = 0.01, onChange, format, defaultValue }: SliderProps) {
  const [local, setLocal] = useLocalValue(value)
  const send = useThrottle(onChange)
  const set = (v: number) => {
    setLocal(v)
    send(v)
  }
  return (
    <label className="slider">
      <span className="slider-label">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={local}
        onChange={(e) => set(Number(e.target.value))}
        onDoubleClick={() => defaultValue !== undefined && set(defaultValue)}
      />
      <span className="slider-value">{format ? format(local) : local}</span>
    </label>
  )
}

export function Toggle({ on, onClick, children, kind }: { on: boolean; onClick: () => void; children: React.ReactNode; kind?: 'mute' | 'solo' }) {
  return (
    <button className={`toggle ${on ? 'on' : ''} ${kind ?? ''}`} onClick={onClick}>
      {children}
    </button>
  )
}

export const fmtDb = (v: number) => (v <= -60 ? '-∞' : `${v > 0 ? '+' : ''}${v.toFixed(1)}`)
export const fmtPan = (p: number) => {
  const v = Math.round(p * 100)
  return v === 0 ? 'C' : v < 0 ? `L${-v}` : `R${v}`
}
export const fmtHz = (v: number) => (v >= 1000 ? `${(v / 1000).toFixed(1)}k` : `${Math.round(v)}`)
export const fmtPct = (v: number) => `${Math.round(v * 100)}%`
export const fmtSec = (v: number) => (v < 1 ? `${Math.round(v * 1000)}ms` : `${v.toFixed(2)}s`)
