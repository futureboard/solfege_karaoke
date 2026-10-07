import { useState } from 'react'
import { act, noteName } from '../api'
import { FileBrowser } from '../components/FileBrowser'
import { MiniMeter, Slider, Toggle, fmtDb, fmtPan, fmtPct, fmtSec } from '../components/controls'
import type { AppState, LoopMode, Meters, SlotParamsView, SlotView } from '../types'

const GM_DRUMS: Record<number, string> = {
  27: 'High Q', 28: 'Slap', 29: 'Scratch Push', 30: 'Scratch Pull', 31: 'Sticks', 32: 'Square Click',
  33: 'Metronome Click', 34: 'Metronome Bell', 35: 'Acoustic Bass Drum', 36: 'Bass Drum 1', 37: 'Side Stick',
  38: 'Acoustic Snare', 39: 'Hand Clap', 40: 'Electric Snare', 41: 'Low Floor Tom', 42: 'Closed Hi-Hat',
  43: 'High Floor Tom', 44: 'Pedal Hi-Hat', 45: 'Low Tom', 46: 'Open Hi-Hat', 47: 'Low-Mid Tom',
  48: 'Hi-Mid Tom', 49: 'Crash Cymbal 1', 50: 'High Tom', 51: 'Ride Cymbal 1', 52: 'Chinese Cymbal',
  53: 'Ride Bell', 54: 'Tambourine', 55: 'Splash Cymbal', 56: 'Cowbell', 57: 'Crash Cymbal 2',
  58: 'Vibraslap', 59: 'Ride Cymbal 2', 60: 'Hi Bongo', 61: 'Low Bongo', 62: 'Mute Hi Conga',
  63: 'Open Hi Conga', 64: 'Low Conga', 65: 'High Timbale', 66: 'Low Timbale', 67: 'High Agogo',
  68: 'Low Agogo', 69: 'Cabasa', 70: 'Maracas', 71: 'Short Whistle', 72: 'Long Whistle', 73: 'Short Guiro',
  74: 'Long Guiro', 75: 'Claves', 76: 'Hi Wood Block', 77: 'Low Wood Block', 78: 'Mute Cuica',
  79: 'Open Cuica', 80: 'Mute Triangle', 81: 'Open Triangle', 82: 'Shaker', 83: 'Jingle Bell',
  84: 'Belltree', 85: 'Castanets', 86: 'Mute Surdo', 87: 'Open Surdo',
}

const OMNI = 0xffff
const NO_DRUMS = OMNI & ~(1 << 9)

function ChannelMask({ slot }: { slot: SlotView }) {
  const mask = slot.params.channels
  const set = (channels: number) => act({ type: 'set_slot', slot: slot.id, patch: { channels } })
  return (
    <div className="channel-mask">
      <div className="mask-buttons">
        {Array.from({ length: 16 }, (_, c) => (
          <button
            key={c}
            className={`ch ${mask & (1 << c) ? 'on' : ''} ${c === 9 ? 'drum' : ''}`}
            onClick={() => set(mask ^ (1 << c))}
            title={`MIDI channel ${c + 1}`}
          >
            {c + 1}
          </button>
        ))}
      </div>
      <div className="mask-presets">
        <button onClick={() => set(OMNI)}>Omni</button>
        <button onClick={() => set(NO_DRUMS)}>No drums (1-9, 11-16)</button>
        <button onClick={() => set(1 << 9)}>Drums only (10)</button>
        <button onClick={() => set(0)}>None</button>
        <span className="dim">receives: {slot.params.channels_text}</span>
      </div>
    </div>
  )
}

function NoteGroupEditor({ slot }: { slot: SlotView }) {
  const g = slot.groups
  const send = (patch: Partial<{ enabled: boolean; map: number[]; names: string[] }>) =>
    act({ type: 'set_note_groups', slot: slot.id, enabled: g.enabled, map: g.map, names: g.names, ...patch })
  const [names, setNames] = useState(g.names)
  const notes = Array.from({ length: 61 }, (_, i) => 27 + i)
  return (
    <div className="note-groups">
      <div className="row">
        <Toggle on={g.enabled} onClick={() => send({ enabled: !g.enabled })}>
          Ch 10 note groups → multi out
        </Toggle>
        <span className="dim">each group gets its own mixer strip (Mixer page)</span>
      </div>
      <div className="group-names">
        {names.map((n, i) => (
          <input
            key={i}
            value={n}
            onChange={(e) => setNames(names.map((x, j) => (j === i ? e.target.value : x)))}
            onBlur={() => send({ names })}
          />
        ))}
      </div>
      <div className="drum-map">
        {notes.map((n) => (
          <label key={n} className="drum-note">
            <span className="mono">{noteName(n)}</span>
            <span className="drum-name">{GM_DRUMS[n] ?? `Note ${n}`}</span>
            <select
              value={g.map[n] >= 8 ? 255 : g.map[n]}
              onChange={(e) => send({ map: g.map.map((v, j) => (j === n ? Number(e.target.value) : v)) })}
            >
              {g.names.map((name, i) => (
                <option key={i} value={i}>
                  {name}
                </option>
              ))}
              <option value={255}>— ch 10 strip</option>
            </select>
          </label>
        ))}
      </div>
    </div>
  )
}

function WavEditor({ slot }: { slot: SlotView }) {
  const w = slot.wav!
  const set = (patch: Partial<typeof w>) => act({ type: 'set_wav', slot: slot.id, patch })
  const time = (v: number) => Math.pow(v, 3) * 10
  const untime = (v: number) => Math.cbrt(v / 10)
  return (
    <div className="grid-params">
      <Slider label="Root" value={w.root} min={0} max={127} step={1} onChange={(v) => set({ root: v })} format={noteName} />
      <label className="slider">
        <span className="slider-label">Loop</span>
        <select value={w.loop_mode} onChange={(e) => set({ loop_mode: e.target.value as LoopMode })}>
          <option value="no_loop">no loop</option>
          <option value="one_shot">one shot</option>
          <option value="loop_continuous">loop</option>
          <option value="loop_sustain">loop sustain</option>
        </select>
        <Toggle on={w.keytrack} onClick={() => set({ keytrack: !w.keytrack })}>
          keytrack
        </Toggle>
      </label>
      {(['attack', 'hold', 'decay', 'release'] as const).map((k) => (
        <Slider
          key={k}
          label={k[0].toUpperCase() + k.slice(1)}
          value={untime(w[k])}
          min={0}
          max={1}
          step={0.001}
          onChange={(v) => set({ [k]: time(v) })}
          format={(v) => fmtSec(time(v))}
        />
      ))}
      <Slider label="Sustain" value={w.sustain} min={0} max={1} onChange={(v) => set({ sustain: v })} format={fmtPct} />
    </div>
  )
}

function SlotCard({ slot, meters, onReplace }: { slot: SlotView; meters: Meters | null; onReplace: () => void }) {
  const [open, setOpen] = useState(true)
  const p = slot.params
  const set = (patch: Partial<Omit<SlotParamsView, 'channels_text'>>) => act({ type: 'set_slot', slot: slot.id, patch })
  const m = meters?.slots.find((x) => x.id === slot.id)
  const receivesDrums = (p.channels & (1 << 9)) !== 0
  return (
    <div className={`slot-card ${p.mute ? 'muted' : ''}`}>
      <div className="slot-head" onClick={() => setOpen(!open)}>
        <span className="slot-index">{slot.index + 1}</span>
        <span className={`kind kind-${slot.kind}`}>{slot.kind}</span>
        <div className="slot-title">
          <strong>{slot.name}</strong>
          <span className="dim">
            {slot.kind === 'SF2' ? slot.preset_name : `${slot.zones} zones`} · {slot.presets} presets · {slot.sample_mb.toFixed(1)} MB
          </span>
        </div>
        <MiniMeter level={Math.max(m?.peak[0] ?? 0, m?.peak[1] ?? 0)} />
        <span className="dim">{m?.voices ?? 0} vc</span>
        <div className="slot-actions" onClick={(e) => e.stopPropagation()}>
          <Toggle kind="mute" on={p.mute} onClick={() => set({ mute: !p.mute })}>M</Toggle>
          <Toggle kind="solo" on={p.solo} onClick={() => set({ solo: !p.solo })}>S</Toggle>
          <button onClick={onReplace}>Replace</button>
          <button className="danger" onClick={() => confirm(`Remove ${slot.name}?`) && act({ type: 'remove_slot', slot: slot.id })}>
            Remove
          </button>
        </div>
      </div>
      {open && (
        <div className="slot-body">
          <div className="path dim">{slot.path}</div>
          {slot.warnings.length > 0 && (
            <ul className="warnings">
              {slot.warnings.map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          )}
          <h5>MIDI channels</h5>
          <ChannelMask slot={slot} />
          <h5>Parameters</h5>
          <div className="grid-params">
            <Slider label="Volume" value={p.volume_db} min={-60} max={12} step={0.5} onChange={(v) => set({ volume_db: v })} format={fmtDb} defaultValue={0} />
            <Slider label="Pan" value={p.pan} min={-1} max={1} onChange={(v) => set({ pan: v })} format={fmtPan} defaultValue={0} />
            <Slider label="Transpose" value={p.transpose} min={-24} max={24} step={1} onChange={(v) => set({ transpose: v })} format={(v) => `${v > 0 ? '+' : ''}${v} st`} defaultValue={0} />
            <Slider label="Fine tune" value={p.tune} min={-100} max={100} step={1} onChange={(v) => set({ tune: v })} format={(v) => `${v} ct`} defaultValue={0} />
            <Slider label="Key low" value={p.key_lo} min={0} max={127} step={1} onChange={(v) => set({ key_lo: v })} format={noteName} defaultValue={0} />
            <Slider label="Key high" value={p.key_hi} min={0} max={127} step={1} onChange={(v) => set({ key_hi: v })} format={noteName} defaultValue={127} />
            <Slider label="Vel low" value={p.vel_lo} min={1} max={127} step={1} onChange={(v) => set({ vel_lo: v })} defaultValue={1} />
            <Slider label="Vel high" value={p.vel_hi} min={1} max={127} step={1} onChange={(v) => set({ vel_hi: v })} defaultValue={127} />
            <Slider label="Bend range" value={p.bend_range} min={0} max={24} step={1} onChange={(v) => set({ bend_range: v })} format={(v) => `±${v}`} defaultValue={2} />
          </div>
          {slot.wav && (
            <>
              <h5>WAV sampler</h5>
              <WavEditor slot={slot} />
            </>
          )}
          {receivesDrums && (
            <>
              <h5>Drums (channel 10)</h5>
              <NoteGroupEditor slot={slot} />
            </>
          )}
        </div>
      )}
    </div>
  )
}

export function RackPage({ state, meters }: { state: AppState; meters: Meters | null }) {
  const [picker, setPicker] = useState<{ replace?: number } | null>(null)
  const full = state.slots.length >= state.max_slots
  return (
    <div className="rack-page">
      <div className="toolbar">
        <button className="primary" disabled={full} onClick={() => setPicker({})}>
          + Add instrument
        </button>
        <span className="dim">
          {state.slots.length}/{state.max_slots} slots · WAV sampler, SFZ, SF2
        </span>
        {state.loading.map((l) => (
          <span key={l} className="loading">loading {l.split(/[\\/]/).pop()}…</span>
        ))}
      </div>
      {state.slots.length === 0 && <div className="empty-page">Rack is empty — add a .wav, .sfz or .sf2.</div>}
      {state.slots.map((s) => (
        <SlotCard key={s.id} slot={s} meters={meters} onReplace={() => setPicker({ replace: s.id })} />
      ))}
      {picker && (
        <FileBrowser
          kind="instrument"
          title={picker.replace ? 'Replace instrument' : 'Add instrument'}
          onClose={() => setPicker(null)}
          onPick={(path) =>
            picker.replace
              ? act({ type: 'replace_instrument', slot: picker.replace, path })
              : act({ type: 'add_instrument', path })
          }
        />
      )}
    </div>
  )
}
