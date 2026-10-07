import { useState } from 'react'
import { act, fmtTime } from '../api'
import type { AppState, Meters } from '../types'
import { PLAY_STATE } from '../types'
import { FileBrowser } from './FileBrowser'
import { Toggle } from './controls'

/** MIDI file player transport, progress and per-channel activity/mutes. */
export function PlayerBar({ state, meters }: { state: AppState; meters: Meters | null }) {
  const [picking, setPicking] = useState(false)
  const song = state.player.song
  const playState = PLAY_STATE[meters?.player.state ?? 0]
  const time = meters?.player.time ?? 0
  const frac = song && song.duration > 0 ? Math.min(1, time / song.duration) : 0
  const mutes = state.player.mutes
  return (
    <div className="player-bar">
      <div className="transport">
        <button onClick={() => setPicking(true)} title="Open MIDI file">
          📂
        </button>
        <button disabled={!song} onClick={() => act({ type: 'transport', op: 'stop' })} title="Stop">
          ■
        </button>
        <button
          className={playState === 'playing' ? 'playing' : ''}
          disabled={!song}
          onClick={() => act({ type: 'transport', op: 'toggle' })}
          title="Play / pause"
        >
          {playState === 'playing' ? '❚❚' : '▶'}
        </button>
        <Toggle on={state.player.looping} onClick={() => act({ type: 'set_loop', on: !state.player.looping })}>
          loop
        </Toggle>
        <label className="speed">
          speed
          <input
            type="range"
            min={0.25}
            max={2}
            step={0.05}
            value={state.player.speed}
            onChange={(e) => act({ type: 'set_speed', speed: Number(e.target.value) })}
            onDoubleClick={() => act({ type: 'set_speed', speed: 1 })}
          />
          {Math.round(state.player.speed * 100)}%
        </label>
        <Toggle on={state.forward_player} onClick={() => act({ type: 'set_forward', on: !state.forward_player })}>
          → MIDI out
        </Toggle>
      </div>
      <div className="song">
        <div className="song-title">
          <strong>{song ? song.name : 'no song'}</strong>
          {song && (
            <span className="dim">
              {' '}
              {fmtTime(time)} / {song.duration_text} · {Math.round(song.bpm * state.player.speed)} BPM · SMF {song.format} · {song.tracks} tracks
            </span>
          )}
        </div>
        <div
          className="progress"
          onClick={(e) => {
            if (!song) return
            const r = e.currentTarget.getBoundingClientRect()
            act({ type: 'seek', time: ((e.clientX - r.left) / r.width) * song.duration })
          }}
        >
          <div className="progress-fill" style={{ width: `${frac * 100}%` }} />
        </div>
      </div>
      <div className="activity">
        {Array.from({ length: 16 }, (_, c) => {
          const used = song ? (song.channels_used & (1 << c)) !== 0 : false
          const muted = (mutes & (1 << c)) !== 0
          const level = meters?.activity[c] ?? 0
          return (
            <button
              key={c}
              className={`act-ch ${used ? 'used' : ''} ${muted ? 'muted' : ''} ${c === 9 ? 'drum' : ''}`}
              onClick={() => act({ type: 'set_mutes', mutes: mutes ^ (1 << c) })}
              title={`channel ${c + 1}: click to ${muted ? 'unmute' : 'mute'}`}
            >
              <span className="act-bar" style={{ height: `${level * 100}%` }} />
              <span className="act-num">{c + 1}</span>
            </button>
          )
        })}
      </div>
      {picking && (
        <FileBrowser kind="song" title="Open MIDI file" onClose={() => setPicking(false)} onPick={(path) => act({ type: 'load_song', path })} />
      )}
    </div>
  )
}
