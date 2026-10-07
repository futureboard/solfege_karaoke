import { useState } from 'react'
import { act, outputName } from '../api'
import { Fader, Knob, Meter, Toggle, fmtDb, fmtHz, fmtPan, fmtPct } from '../components/controls'
import type { AppState, FxParams, Meters, SlotMeters, SlotView, StripParams, StripView } from '../types'

const DEFAULT_STRIP: StripParams = {
  gain_db: 0, pan: 0, mute: false, solo: false, eq_low_db: 0, eq_mid_db: 0, eq_mid_hz: 1000, eq_high_db: 0,
  hpf_hz: 20, lpf_hz: 20000, reverb: 1, chorus: 1, output: 0,
}

const edited = (p: StripParams) =>
  (Object.keys(DEFAULT_STRIP) as (keyof StripParams)[]).some((k) => p[k] !== DEFAULT_STRIP[k])

/** Hide idle strips: show ones with signal, used by the song, programmed or edited. */
function isActive(state: AppState, st: StripView, sm: SlotMeters | undefined) {
  if (((sm?.live ?? 0) & (1 << st.index)) !== 0) return true
  const song = state.player.song
  if (song && (song.channels_used & (1 << st.channel)) !== 0) return true
  if (st.group === null && sm?.channels[st.channel]?.explicit) return true
  return edited(st.params)
}

function Strip({ slot, strip, level, live, outPairs, maxBuses }: {
  slot: SlotView
  strip: StripView
  level: [number, number]
  live: boolean
  outPairs: number
  maxBuses: number
}) {
  const p = strip.params
  const set = (patch: Partial<StripParams>) =>
    act({ type: 'set_strip', slot: slot.id, strip: strip.index, params: { ...p, ...patch } })
  const isGroup = strip.group !== null
  return (
    <div className={`strip ${isGroup ? 'group' : ''} ${live ? 'live' : ''} ${p.mute ? 'muted' : ''}`}>
      <div className="strip-name" title={strip.name}>
        <span className="strip-ch">{isGroup ? `10·${strip.group! + 1}` : `CH ${strip.channel + 1}`}</span>
        <span>{strip.name.replace(/^\s*\d+\s*/, '')}</span>
      </div>
      <div className="strip-section">
        <Knob label="Low" value={p.eq_low_db} min={-18} max={18} defaultValue={0} format={fmtDb} onChange={(v) => set({ eq_low_db: v })} />
        <Knob label="Mid" value={p.eq_mid_db} min={-18} max={18} defaultValue={0} format={fmtDb} onChange={(v) => set({ eq_mid_db: v })} />
        <Knob label="Freq" value={p.eq_mid_hz} min={100} max={10000} log defaultValue={1000} format={fmtHz} onChange={(v) => set({ eq_mid_hz: v })} />
        <Knob label="High" value={p.eq_high_db} min={-18} max={18} defaultValue={0} format={fmtDb} onChange={(v) => set({ eq_high_db: v })} />
      </div>
      <div className="strip-section">
        <Knob label="HPF" value={p.hpf_hz} min={20} max={2000} log defaultValue={20} format={(v) => (v <= 21 ? 'off' : fmtHz(v))} onChange={(v) => set({ hpf_hz: v })} />
        <Knob label="LPF" value={p.lpf_hz} min={500} max={20000} log defaultValue={20000} format={(v) => (v >= 19900 ? 'off' : fmtHz(v))} onChange={(v) => set({ lpf_hz: v })} />
      </div>
      <div className="strip-section">
        <Knob label="Rev" value={p.reverb} min={0} max={2} defaultValue={1} format={fmtPct} onChange={(v) => set({ reverb: v })} />
        <Knob label="Cho" value={p.chorus} min={0} max={2} defaultValue={1} format={fmtPct} onChange={(v) => set({ chorus: v })} />
      </div>
      <Knob label="Pan" value={p.pan} min={-1} max={1} defaultValue={0} format={fmtPan} onChange={(v) => set({ pan: v })} />
      <div className="strip-ms">
        <Toggle kind="mute" on={p.mute} onClick={() => set({ mute: !p.mute })}>M</Toggle>
        <Toggle kind="solo" on={p.solo} onClick={() => set({ solo: !p.solo })}>S</Toggle>
      </div>
      <div className="strip-fader">
        <Meter level={level} size={150} />
        <Fader value={p.gain_db} min={-60} max={12} step={0.5} defaultValue={0} height={150} onChange={(v) => set({ gain_db: v })} />
      </div>
      <div className="strip-db">{fmtDb(p.gain_db)}</div>
      <select className="strip-out" value={p.output} onChange={(e) => set({ output: Number(e.target.value) })}>
        {Array.from({ length: maxBuses }, (_, b) => (
          <option key={b} value={b}>
            {outputName(b)}
            {b >= outPairs ? ' (→Main)' : ''}
          </option>
        ))}
      </select>
    </div>
  )
}

function FxStrip({ fx, kind, level }: { fx: FxParams; kind: 'reverb' | 'chorus'; level: number }) {
  const set = (patch: Partial<FxParams>) => act({ type: 'set_fx', fx: { ...fx, ...patch } })
  return (
    <div className="strip fx">
      <div className="strip-name">
        <span className="strip-ch">FX</span>
        <span>{kind === 'reverb' ? 'Reverb' : 'Chorus'}</span>
      </div>
      {kind === 'reverb' ? (
        <div className="strip-section column">
          <Knob label="Room" value={fx.reverb_room} min={0} max={1} defaultValue={0.6} format={fmtPct} onChange={(v) => set({ reverb_room: v })} />
          <Knob label="Damp" value={fx.reverb_damp} min={0} max={1} defaultValue={0.4} format={fmtPct} onChange={(v) => set({ reverb_damp: v })} />
          <Knob label="Width" value={fx.reverb_width} min={0} max={1} defaultValue={1} format={fmtPct} onChange={(v) => set({ reverb_width: v })} />
        </div>
      ) : (
        <div className="strip-section column">
          <Knob label="Rate" value={fx.chorus_rate} min={0.05} max={8} log defaultValue={0.8} format={(v) => `${v.toFixed(2)}Hz`} onChange={(v) => set({ chorus_rate: v })} />
          <Knob label="Depth" value={fx.chorus_depth} min={0} max={15} defaultValue={3} format={(v) => `${v.toFixed(1)}ms`} onChange={(v) => set({ chorus_depth: v })} />
          <Knob label="Delay" value={fx.chorus_delay} min={2} max={30} defaultValue={12} format={(v) => `${v.toFixed(1)}ms`} onChange={(v) => set({ chorus_delay: v })} />
        </div>
      )}
      <div className="strip-fader">
        <Meter level={[level, level]} size={150} />
        <Fader
          value={kind === 'reverb' ? fx.reverb_return : fx.chorus_return}
          min={0}
          max={2}
          step={0.01}
          height={150}
          defaultValue={0.5}
          onChange={(v) => set(kind === 'reverb' ? { reverb_return: v } : { chorus_return: v })}
        />
      </div>
      <div className="strip-db">return {fmtPct(kind === 'reverb' ? fx.reverb_return : fx.chorus_return)}</div>
    </div>
  )
}

export function MixerPage({ state, meters }: { state: AppState; meters: Meters | null }) {
  const outPairs = meters?.out_pairs ?? 1
  const [showAll, setShowAll] = useState(false)
  return (
    <div className="mixer-page">
      <div className="toolbar">
        <Toggle on={!showAll} onClick={() => setShowAll(!showAll)}>
          active strips only
        </Toggle>
        <span className="dim">
          one strip per MIDI channel · channel 10 split by note group (Rack Editor → Drums) · outputs: {outPairs} stereo pair(s)
        </span>
      </div>
      <div className="console">
        {state.slots.map((slot) => {
          const sm = meters?.slots.find((m) => m.id === slot.id)
          return (
            <section key={slot.id} className="console-slot">
              <header>
                <span className={`kind kind-${slot.kind}`}>{slot.kind}</span> {slot.name}
                <span className="dim"> · {slot.params.channels_text}</span>
              </header>
              <div className="strips">
                {slot.strips.filter((st) => showAll || isActive(state, st, sm)).map((st) => (
                  <Strip
                    key={st.index}
                    slot={slot}
                    strip={st}
                    level={sm?.strips[st.index] ?? [0, 0]}
                    live={((sm?.live ?? 0) & (1 << st.index)) !== 0}
                    outPairs={outPairs}
                    maxBuses={state.max_buses}
                  />
                ))}
                {slot.strips.length === 0 && <div className="empty">no channels received</div>}
                {slot.strips.length > 0 && !showAll && !slot.strips.some((st) => isActive(state, st, sm)) && (
                  <div className="empty">idle — play something or show all</div>
                )}
              </div>
            </section>
          )
        })}
        <section className="console-slot returns">
          <header>FX returns · Outputs</header>
          <div className="strips">
            <FxStrip fx={state.fx} kind="reverb" level={meters?.fx[0] ?? 0} />
            <FxStrip fx={state.fx} kind="chorus" level={meters?.fx[1] ?? 0} />
            {Array.from({ length: outPairs }, (_, b) => (
              <div key={b} className="strip bus">
                <div className="strip-name">
                  <span className="strip-ch">OUT</span>
                  <span>{outputName(b)}</span>
                </div>
                <div className="strip-fader">
                  <Meter level={meters?.buses[b] ?? [0, 0]} size={150} />
                </div>
              </div>
            ))}
            <div className="strip master">
              <div className="strip-name">
                <span className="strip-ch">MASTER</span>
                <span>{state.audio?.device ?? 'no audio'}</span>
              </div>
              <div className="strip-fader">
                <Meter level={meters?.master ?? [0, 0]} size={150} />
                <Fader value={state.master_db} min={-60} max={12} step={0.5} height={150} defaultValue={0} onChange={(db) => act({ type: 'set_master', db })} />
              </div>
              <div className="strip-db">{fmtDb(state.master_db)}</div>
              <button className="danger" onClick={() => act({ type: 'panic' })}>
                PANIC
              </button>
            </div>
          </div>
        </section>
      </div>
    </div>
  )
}
