import { useEffect, useState } from 'react'
import { browse } from '../api'
import type { BrowseResult } from '../types'

interface Props {
  kind: 'instrument' | 'song'
  title: string
  onPick: (path: string) => void
  onClose: () => void
}

/** Server-side file picker (the app reads files from its own disk). */
export function FileBrowser({ kind, title, onPick, onClose }: Props) {
  const [path, setPath] = useState<string | null>(null)
  const [data, setData] = useState<BrowseResult | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [filter, setFilter] = useState('')

  useEffect(() => {
    browse(path, kind)
      .then((d) => {
        setData(d)
        setError(null)
      })
      .catch((e) => setError(String(e)))
  }, [path, kind])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const entries = (data?.entries ?? []).filter((e) => e.name.toLowerCase().includes(filter.toLowerCase()))
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{title}</h3>
          <button onClick={onClose}>✕</button>
        </div>
        <div className="browser-bar">
          <button disabled={!data?.parent} onClick={() => data?.parent && setPath(data.parent)}>
            ↑ Up
          </button>
          {data?.roots.map((r) => (
            <button key={r} onClick={() => setPath(r)}>
              {r}
            </button>
          ))}
          <input placeholder="filter…" value={filter} onChange={(e) => setFilter(e.target.value)} autoFocus />
        </div>
        <div className="browser-path">{data?.dir}</div>
        {error && <div className="error">{error}</div>}
        <ul className="browser-list">
          {entries.map((e) => (
            <li
              key={e.path}
              className={e.dir ? 'dir' : `file ext-${e.name.split('.').pop()?.toLowerCase()}`}
              onClick={() => {
                if (e.dir) {
                  setPath(e.path)
                  setFilter('')
                } else {
                  onPick(e.path)
                  onClose()
                }
              }}
            >
              {e.dir ? '▸ ' : ''}
              {e.name}
            </li>
          ))}
          {entries.length === 0 && <li className="empty">nothing here</li>}
        </ul>
      </div>
    </div>
  )
}
