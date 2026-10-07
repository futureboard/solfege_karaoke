import { useEffect, useMemo, useState } from 'react'
import { act, getPresets } from '../api'
import { MiniMeter } from '../components/controls'
import { Piano, notesMask } from '../components/Piano'
import type { AppState, Meters, PresetInfo } from '../types'

interface Props {
  state: AppState
  meters: Meters | null
  slotId: number | null
  setSlotId: (id: number) => void
}

/** Browse a slot's presets and assign them to the slot or a single channel. */
export function PresetPage({ state, meters, slotId, setSlotId }: Props) {
  const slot = state.slots.find((s) => s.id === slotId) ?? state.slots[0]
  const [presets, setPresets] = useState<PresetInfo[]>([])
  const [filter, setFilter] = useState('')
  const [bank, setBank] = useState<number | 'all'>('all')
  const [target, setTarget] = useState<number | 'slot'>('slot')

  useEffect(() => {
    if (!slot) return
    getPresets(slot.id, slot.path).then(setPresets)
  }, [slot?.id, slot?.path])

  const banks = useMemo(() => [...new Set(presets.map((p) => p.bank))].sort((a, b) => a - b), [presets])
  const shown = presets.filter(
    (p) =>
      (bank === 'all' || p.bank === bank) &&
      (filter === '' ||
        p.name.toLowerCase().includes(filter.toLowerCase()) ||
        `${p.bank}:${p.program}`.includes(filter)),
  )

  if (!slot) return <div className="empty-page">Add an instrument on the Rack page first.</div>
  const sm = meters?.slots.find((m) => m.id === slot.id)
  const current = target === 'slot' ? slot.preset : sm?.channels[target]?.preset

  const assign = (p: PresetInfo) => {
    if (target === 'slot') act({ type: 'set_preset', slot: slot.id, preset: p.index })
    else act({ type: 'set_channel_preset', slot: slot.id, channel: target, preset: p.index })
  }
  const receives = (c: number) => (slot.params.channels & (1 << c)) !== 0

  return (
    <div className="preset-page">
      <aside className="slot-list">
        <h4>Slots</h4>
        {state.slots.map((s) => {
          const m = meters?.slots.find((x) => x.id === s.id)
          return (
            <button key={s.id} className={`slot-pick ${s.id === slot.id ? 'active' : ''}`} onClick={() => setSlotId(s.id)}>
              <span className={`kind kind-${s.kind}`}>{s.kind}</span>
              <span className="slot-pick-name">{s.name}</span>
              <MiniMeter level={Math.max(m?.peak[0] ?? 0, m?.peak[1] ?? 0)} />
            </button>
          )
        })}
      </aside>

      <section className="preset-browser">
        <div className="toolbar">
          <input placeholder="Search presets…" value={filter} onChange={(e) => setFilter(e.target.value)} />
          <div className="chips">
            <button className={bank === 'all' ? 'chip on' : 'chip'} onClick={() => setBank('all')}>
              All banks
            </button>
            {banks.map((b) => (
              <button key={b} className={bank === b ? 'chip on' : 'chip'} onClick={() => setBank(b)}>
                {b === 128 ? 'Drums 128' : `Bank ${b}`}
              </button>
            ))}
          </div>
        </div>
        <div className="assign-target">
          Assign to:{' '}
          <strong>{target === 'slot' ? `slot default (${slot.params.channels_text})` : `channel ${target + 1} only`}</strong>
          {target !== 'slot' && <button onClick={() => setTarget('slot')}>use slot default</button>}
        </div>
        <div className="preset-table">
          {shown.map((p) => (
            <div key={p.index} className={`preset-row ${p.index === current ? 'current' : ''}`} onClick={() => assign(p)}>
              <span className="mono">
                {String(p.bank).padStart(3, '0')}:{String(p.program).padStart(3, '0')}
              </span>
              <span className="preset-name">{p.name}</span>
              <span className="dim">{p.zones} zones</span>
            </div>
          ))}
          {shown.length === 0 && <div className="empty">no presets match</div>}
        </div>
      </section>

      <section className="channel-map">
        <h4>Channels · {slot.params.channels_text}</h4>
        <table>
          <thead>
            <tr>
              <th>Ch</th>
              <th>Bank</th>
              <th>Prg</th>
              <th>Preset</th>
              <th></th>
              <th>Vc</th>
            </tr>
          </thead>
          <tbody>
            {Array.from({ length: 16 }, (_, c) => {
              const info = sm?.channels[c]
              const p = presets[info?.preset ?? -1]
              const rx = receives(c)
              const source = !rx
                ? '—'
                : info?.locked
                  ? 'locked'
                  : info?.fallback
                  ? 'fallback'
                  : info?.explicit
                    ? 'program'
                    : info?.drum
                      ? 'drums'
                      : 'default'
              return (
                <tr
                  key={c}
                  className={`${target === c ? 'selected' : ''} ${rx ? '' : 'off'} ${info && info.voices > 0 ? 'busy' : ''}`}
                  onClick={() => rx && setTarget(c)}
                >
                  <td>{c + 1}</td>
                  <td className="mono">
                    {info ? `${info.msb}:${info.lsb}` : ''}
                  </td>
                  <td className="mono">{info?.program ?? '—'}</td>
                  <td>{p ? p.name : ''}</td>
                  <td>
                    <span className={`badge ${source}`}>{source}</span>
                    {info?.explicit && rx && (
                      <button
                        className="tiny"
                        title="unpin / back to default"
                        onClick={(e) => {
                          e.stopPropagation()
                          act({ type: 'set_channel_preset', slot: slot.id, channel: c, preset: null })
                        }}
                      >
                        ✕
                      </button>
                    )}
                  </td>
                  <td>{info?.voices || ''}</td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </section>

      <footer className="preset-piano">
        <Piano slot={slot.id} active={notesMask(sm?.notes)} />
      </footer>
    </div>
  )
}
