import { useEffect, useState } from 'react'
import { useBackend } from './api'
import { Meter } from './components/controls'
import { PlayerBar } from './components/PlayerBar'
import { MixerPage } from './pages/MixerPage'
import { PresetPage } from './pages/PresetPage'
import { RackPage } from './pages/RackPage'

type Page = 'presets' | 'rack' | 'mixer'

const PAGES: { id: Page; label: string }[] = [
  { id: 'presets', label: 'Presets' },
  { id: 'rack', label: 'Rack Editor' },
  { id: 'mixer', label: 'Mixer' },
]

export default function App() {
  const { state, meters, connected } = useBackend()
  const [page, setPage] = useState<Page>(() => (location.hash.slice(1) as Page) || 'rack')
  const [slotId, setSlotId] = useState<number | null>(null)
  useEffect(() => {
    const onHash = () => {
      const h = location.hash.slice(1) as Page
      if (PAGES.some((p) => p.id === h)) setPage(h)
    }
    window.addEventListener('hashchange', onHash)
    return () => window.removeEventListener('hashchange', onHash)
  }, [])
  const go = (p: Page) => {
    setPage(p)
    location.hash = p
  }

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          simpletui <span className="dim">instrument rack</span>
        </div>
        <nav>
          {PAGES.map((p) => (
            <button key={p.id} className={page === p.id ? 'tab active' : 'tab'} onClick={() => go(p.id)}>
              {p.label}
            </button>
          ))}
        </nav>
        <div className="status">
          <span className={`dot ${connected ? 'ok' : 'bad'}`} title={connected ? 'connected' : 'disconnected'} />
          {state?.audio ? (
            <span className="dim">
              {state.audio.host} · {state.audio.device} · {state.audio.sample_rate} Hz
            </span>
          ) : (
            <span className="error">{state?.audio_error ?? 'audio not running'}</span>
          )}
          <span>CPU {Math.round((meters?.cpu ?? 0) * 100)}%</span>
          <span>{meters?.voices ?? 0} voices</span>
          <Meter level={meters?.master ?? [0, 0]} vertical={false} size={110} />
        </div>
      </header>

      {!state ? (
        <div className="empty-page">{connected ? 'Loading…' : 'Connecting to simpletui…'}</div>
      ) : (
        <>
          <PlayerBar state={state} meters={meters} />
          <main>
            {page === 'presets' && <PresetPage state={state} meters={meters} slotId={slotId} setSlotId={setSlotId} />}
            {page === 'rack' && <RackPage state={state} meters={meters} />}
            {page === 'mixer' && <MixerPage state={state} meters={meters} />}
          </main>
          <footer className="logbar">
            {state.log.slice(-3).map((l, i) => (
              <div key={i} className={l.error ? 'error' : 'dim'}>
                {l.text}
              </div>
            ))}
          </footer>
        </>
      )}
    </div>
  )
}
