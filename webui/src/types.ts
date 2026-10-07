// Mirrors the JSON produced by src/app/web.rs and src/web.rs.

export interface StripParams {
  gain_db: number
  pan: number
  mute: boolean
  solo: boolean
  eq_low_db: number
  eq_mid_db: number
  eq_mid_hz: number
  eq_high_db: number
  hpf_hz: number
  lpf_hz: number
  reverb: number
  chorus: number
  output: number
}

export interface FxParams {
  reverb_return: number
  reverb_room: number
  reverb_damp: number
  reverb_width: number
  chorus_return: number
  chorus_rate: number
  chorus_depth: number
  chorus_delay: number
}

export interface StripView {
  index: number
  name: string
  channel: number
  group: number | null
  params: StripParams
}

export interface SlotParamsView {
  volume_db: number
  pan: number
  channels: number
  channels_text: string
  transpose: number
  tune: number
  key_lo: number
  key_hi: number
  vel_lo: number
  vel_hi: number
  bend_range: number
  mute: boolean
  solo: boolean
}

export type LoopMode = 'no_loop' | 'one_shot' | 'loop_continuous' | 'loop_sustain'

export interface WavView {
  root: number
  keytrack: boolean
  loop_mode: LoopMode
  attack: number
  hold: number
  decay: number
  sustain: number
  release: number
}

export interface SlotView {
  id: number
  index: number
  name: string
  kind: 'WAV' | 'SFZ' | 'SF2'
  path: string
  preset: number
  preset_name: string
  presets: number
  zones: number
  sample_mb: number
  warnings: string[]
  params: SlotParamsView
  wav: WavView | null
  strips: StripView[]
  groups: { enabled: boolean; map: number[]; names: string[] }
}

export interface SongView {
  name: string
  path: string
  duration: number
  duration_text: string
  bpm: number
  format: number
  tracks: number
  events: number
  channels_used: number
}

export interface AppState {
  audio: { host: string; device: string; sample_rate: number; channels: number; format: string; buffer: string } | null
  audio_error: string | null
  midi_inputs: string[]
  midi_output: string | null
  midi_thru: boolean
  forward_player: boolean
  master_db: number
  fx: FxParams
  max_buses: number
  max_slots: number
  slots: SlotView[]
  player: { song: SongView | null; looping: boolean; speed: number; mutes: number }
  loading: string[]
  log: { text: string; error: boolean }[]
  browse_dir: string
}

export interface ChannelMeter {
  preset: number
  program: number | null
  msb: number
  lsb: number
  volume: number
  pan: number
  expression: number
  bend: number
  sustain: boolean
  drum: boolean
  voices: number
  explicit: boolean
  fallback: boolean
  locked: boolean
}

export interface SlotMeters {
  id: number
  peak: [number, number]
  voices: number
  live: number
  notes: [string, string]
  strips: [number, number][]
  channels: ChannelMeter[]
}

export interface Meters {
  type: 'meters'
  version: number
  cpu: number
  voices: number
  master: [number, number]
  buses: [number, number][]
  fx: [number, number]
  out_pairs: number
  player: { state: number; time: number }
  activity: number[]
  slots: SlotMeters[]
}

export interface PresetInfo {
  index: number
  bank: number
  program: number
  name: string
  zones: number
}

export interface BrowseResult {
  dir: string
  parent: string | null
  roots: string[]
  entries: { name: string; path: string; dir: boolean }[]
}

export type Action =
  | { type: 'add_instrument'; path: string }
  | { type: 'replace_instrument'; slot: number; path: string }
  | { type: 'remove_slot'; slot: number }
  | { type: 'set_slot'; slot: number; patch: Partial<Omit<SlotParamsView, 'channels_text'>> }
  | { type: 'set_wav'; slot: number; patch: Partial<WavView> }
  | { type: 'set_preset'; slot: number; preset: number }
  | { type: 'set_channel_preset'; slot: number; channel: number; preset: number | null }
  | { type: 'set_strip'; slot: number; strip: number; params: StripParams }
  | { type: 'set_note_groups'; slot: number; enabled: boolean; map: number[]; names: string[] }
  | { type: 'set_fx'; fx: FxParams }
  | { type: 'set_master'; db: number }
  | { type: 'transport'; op: 'play' | 'pause' | 'toggle' | 'stop' }
  | { type: 'seek'; time: number }
  | { type: 'set_loop'; on: boolean }
  | { type: 'set_speed'; speed: number }
  | { type: 'set_mutes'; mutes: number }
  | { type: 'set_forward'; on: boolean }
  | { type: 'load_song'; path: string }
  | { type: 'note'; slot: number; key: number; vel: number }
  | { type: 'panic' }

export const PLAY_STATE = ['empty', 'stopped', 'playing', 'paused'] as const
