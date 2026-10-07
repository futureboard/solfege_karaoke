import { useRef, useState } from 'react'
import { act, noteName } from '../api'

const BLACK = new Set([1, 3, 6, 8, 10])

interface Props {
  slot: number
  active: bigint
  lo?: number
  octaves?: number
}

/** Clickable keyboard that plays the given slot; lights notes from the engine. */
export function Piano({ slot, active, lo: initialLo = 48, octaves = 3 }: Props) {
  const [lo, setLo] = useState(initialLo)
  const [vel, setVel] = useState(100)
  const held = useRef<Set<number>>(new Set())
  const hi = lo + octaves * 12
  const whites: number[] = []
  for (let n = lo; n <= hi; n++) if (!BLACK.has(n % 12)) whites.push(n)

  const on = (n: number) => {
    if (held.current.has(n)) return
    held.current.add(n)
    act({ type: 'note', slot, key: n, vel })
  }
  const off = (n: number) => {
    if (!held.current.delete(n)) return
    act({ type: 'note', slot, key: n, vel: 0 })
  }
  const lit = (n: number) => ((active >> BigInt(n)) & 1n) === 1n
  const keyProps = (n: number) => ({
    onPointerDown: (e: React.PointerEvent) => {
      ;(e.target as Element).releasePointerCapture(e.pointerId)
      on(n)
    },
    onPointerUp: () => off(n),
    onPointerLeave: () => off(n),
    onPointerEnter: (e: React.PointerEvent) => e.buttons === 1 && on(n),
  })

  return (
    <div className="piano-wrap">
      <div className="piano-controls">
        <button onClick={() => setLo(Math.max(0, lo - 12))}>− oct</button>
        <span>
          {noteName(lo)}–{noteName(hi)}
        </span>
        <button onClick={() => setLo(Math.min(127 - octaves * 12, lo + 12))}>+ oct</button>
        <label>
          vel <input type="range" min={1} max={127} value={vel} onChange={(e) => setVel(Number(e.target.value))} /> {vel}
        </label>
      </div>
      <div className="piano">
        {whites.map((n) => (
          <div key={n} className={`white ${lit(n) ? 'lit' : ''}`} {...keyProps(n)}>
            {n % 12 === 0 && <span>{noteName(n)}</span>}
            {n + 1 <= hi && BLACK.has((n + 1) % 12) && (
              <div
                className={`black ${lit(n + 1) ? 'lit' : ''}`}
                {...keyProps(n + 1)}
                onPointerDown={(e) => {
                  e.stopPropagation()
                  on(n + 1)
                }}
              />
            )}
          </div>
        ))}
      </div>
    </div>
  )
}

/** Combine the two 64-bit note masks sent as decimal strings. */
export function notesMask(notes: [string, string] | undefined): bigint {
  if (!notes) return 0n
  return BigInt(notes[0]) | (BigInt(notes[1]) << 64n)
}
