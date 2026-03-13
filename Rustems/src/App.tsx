import StemSlider from "./components/StemSlider"
import { useStemPlayer, listDeviceTracks, deleteTrack, deleteAlbum } from "./hooks/useStemPlayer"
import type { AlbumInfo } from "./hooks/useStemPlayer"
import { useEffect, useRef, useState } from "react"
import { pickFolder } from "./components/StemSlider"
import { invoke } from "@tauri-apps/api/core"
import { open, message, confirm } from "@tauri-apps/plugin-dialog"

interface StorageInfo {
  total_bytes: number
  free_bytes: number
  used_bytes: number
}

interface SplitResult {
  output_dir: string
  track_name: string 
  stems: string[] // [vocals.mp3, drums.mp3, bass.mp3, other.mp3]
}

function StorageBar({ storage }: { storage: StorageInfo }) {
  const usedPct  = storage.total_bytes > 0
    ? Math.min(100, (storage.used_bytes / storage.total_bytes) * 100)
    : 0
  // Device reports kilobytes — multiply by 1024 to get bytes, then convert to GB
  const toGB = (kb: number) => (kb * 1024 / 1_073_741_824).toFixed(2)
  const barColour = usedPct > 90 ? "#e05555" : usedPct > 70 ? "#e08c30" : "#4caf82"

  return (
    <div style={{ marginTop: 12, marginBottom: 4 }}>
      <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, color: "#888", marginBottom: 5 }}>
        <span>Storage</span>
        <span>{toGB(storage.used_bytes)} GB used · {toGB(storage.free_bytes)} GB free · {toGB(storage.total_bytes)} GB total</span>
      </div>
      <div style={{ height: 6, borderRadius: 3, background: "#2a2a2a", overflow: "hidden" }}>
        <div style={{
          height: "100%", width: `${usedPct}%`,
          background: barColour, borderRadius: 3,
          transition: "width 0.4s ease",
        }} />
      </div>
    </div>
  )
}

// ── Main component ────────────────────────────────────────────────────────────

export default function App() {
  const { loadSong, play, pause } = useStemPlayer()

  const [devices, setDevices]               = useState<string[]>([])
  const [selectedDevice, setSelectedDevice] = useState("")
  const [connected, setConnected]           = useState(false)
  const [albums, setAlbums]                 = useState<AlbumInfo[]>([])
  const [loadingTracks, setLoadingTracks]   = useState(false)
  const [refreshing, setRefreshing]         = useState(false)
  const [storage, setStorage]               = useState<StorageInfo | null>(null)
  const pollRef                             = useRef<ReturnType<typeof setInterval> | null>(null)

  const [folder, setFolder]       = useState<string | null>(null)
  const [trackName, setTrackName] = useState("")
  const [uploading, setUploading] = useState(false)

  const [splitterStatus, setSplitterStatus] = useState<"unknown" | "checking" | "ok" | "missing">("unknown")
  const [splitterDetail, setSplitterDetail] = useState("")
  const [splitting, setSplitting]           = useState(false)
  const [splitResult, setSplitResult]       = useState<SplitResult | null>(null)
  const [splitTrackName, setSplitTrackName] = useState("")


  useEffect(() => {
    invoke<string[]>("list_usb_devices").then(setDevices)
    checkSplitter()

    // Poll for device changes every 2s when not connected
    pollRef.current = setInterval(async () => {
      if (connected) return
      try {
        const found = await invoke<string[]>("list_usb_devices")
        setDevices(prev => {
          const added   = found.filter(d => !prev.includes(d))
          const removed = prev.filter(d => !found.includes(d))
          if (added.length === 0 && removed.length === 0) return prev
          // Auto-select if exactly one device just appeared and nothing is selected
          if (added.length === 1) {
            setSelectedDevice(sel => sel === "" ? added[0] : sel)
          }
          // Clear selection if the selected device was unplugged
          if (removed.length > 0) {
            setSelectedDevice(sel => removed.includes(sel) ? "" : sel)
          }
          return found
        })
      } catch { /* silently ignore scan errors */ }
    }, 2000)

    return () => { if (pollRef.current) clearInterval(pollRef.current) }
  }, [connected])

  const handleRefreshDevices = async () => {
    setRefreshing(true)
    try {
      const found = await invoke<string[]>("list_usb_devices")
      setDevices(found)
      if (found.length === 1) setSelectedDevice(found[0])
    } finally {
      setRefreshing(false)
    }
  }

  const checkSplitter = async () => {
    setSplitterStatus("checking")
    try {
      const detail = await invoke<string>("check_splitter")
      setSplitterDetail(detail)
      setSplitterStatus("ok")
    } catch (err) {
      setSplitterDetail(String(err))
      setSplitterStatus("missing")
    }
  }

  const fetchStorage = async () => {
    try {
      const s = await invoke<StorageInfo>("get_storage_info")
      setStorage(s)
    } catch { /* non-fatal */ }
  }

  // ── Device handlers ──────────────────────────────────────────────────────────

  const handleConnect = async () => {
    if (!selectedDevice) return
    try {
      await invoke<string>("connect_usb_device", { serial: selectedDevice })
      setConnected(true)
      await fetchTracks()
      await fetchStorage()
    } catch (err) {
      await message(`Failed to connect: ${err}`, { title: "Error", kind: "error" })
    }
  }

  const handleDisconnect = async () => {
    try {
      await invoke("disconnect_device")
      setConnected(false)
      setAlbums([])
      setStorage(null)
    } catch (err) {
      await message(`Disconnect failed: ${err}`, { title: "Error", kind: "error" })
    }
  }

  const fetchTracks = async () => {
    setLoadingTracks(true)
    try {
      const result = await listDeviceTracks()
      setAlbums(result)
    } catch (err) {
      await message(`Failed to list tracks: ${err}`, { title: "Error", kind: "error" })
    } finally {
      setLoadingTracks(false)
    }
  }

  // ── Manual upload handlers ────────────────────────────────────────────────────

  const handlePickAndLoad = async () => {
    const selectedFolder = await pickFolder()
    if (!selectedFolder) return
    setFolder(selectedFolder)
    const parts = selectedFolder.replace(/\\/g, "/").split("/")
    setTrackName(parts[parts.length - 1] ?? "")
    await loadSong(selectedFolder)
  }

  const handleUpload = async () => {
    if (!folder) {
      await message("Load a folder first", { title: "Rustems", kind: "warning" })
      return
    }
    if (!connected) {
      await message("Connect a device first", { title: "Rustems", kind: "warning" })
      return
    }
    const yes = await confirm(
      `Upload "${trackName}" to device?`,
      { title: "Confirm Upload", kind: "info" }
    )
    if (!yes) return
    
    setUploading(true)
    try {
      await invoke("upload_stems", { folder, trackName: trackName.trim() || undefined })
      await message("Upload complete!", { title: "Rustems", kind: "info" })
      await fetchTracks()
      await fetchStorage()
    } catch (err) {
      await message(`Upload failed: ${err}`, { title: "Error", kind: "error" })
    } finally {
      setUploading(false)
    }
  }

  // ── Delete handlers ──────────────────────────────────────────────────────────

  const handleDeleteTrack = async (albumId: string, trackId: string, title: string) => {
    const yes = await confirm(`Delete "${title}" from the device? This cannot be undone.`, {
      title: "Delete Track", kind: "warning",
    })
    if (!yes) return
    try {
      await deleteTrack(albumId, trackId)
      await fetchTracks()
      await fetchStorage()
    } catch (err) {
      await message(`Delete failed: ${err}`, { title: "Error", kind: "error" })
    }
  }

  const handleDeleteAlbum = async (albumId: string) => {
    const yes = await confirm(
      `Delete entire album ${albumId} and all its tracks? This cannot be undone.`,
      { title: "Delete Album", kind: "warning" }
    )
    if (!yes) return
    try {
      await deleteAlbum(albumId)
      await fetchTracks()
      await fetchStorage()
    } catch (err) {
      await message(`Delete failed: ${err}`, { title: "Error", kind: "error" })
    }
  }

  // ── Splitter handlers ────────────────────────────────────────────────────────

  const handlePickAndSplit = async () => {
    const selected = await open({
      title: "Select audio file to split",
      multiple: false,
      filters: [{ name: "Audio", extensions: ["mp3", "wav", "flac", "aac", "ogg", "m4a"] }],
    })
    if (!selected || typeof selected !== "string") return

    setSplitting(true)
    setSplitResult(null)
    try {
      const result = await invoke<SplitResult>("split_stems", {
        inputPath: selected,
      })
      setSplitResult(result)
      setSplitTrackName(result.track_name)
      await loadSong(result.output_dir)
    } catch (err) {
      await message(`Splitting failed: ${err}`, { title: "Splitter Error", kind: "error" })
    } finally {
      setSplitting(false)
    }
  }

  const handleUploadSplit = async () => {
    if (!splitResult) return
    if (!connected) {
      await message("Connect a device first", { title: "Rustems", kind: "warning" })
      return
    }
    const yes = await confirm(
      `Upload "${splitTrackName}" to device?\n\nStems: vocals, drums, bass, melody (Splitter's "other" stem)`,
      { title: "Confirm Upload", kind: "info" }
    )
    if (!yes) return

    setUploading(true)
    try {
      await invoke("upload_stems", {
        folder: splitResult.output_dir,
        trackName: splitTrackName.trim() || splitResult.track_name,
      })
      await message("Upload complete!", { title: "Rustems", kind: "info" })
      setSplitResult(null)
      await fetchTracks()
      await fetchStorage()
    } catch (err) {
      await message(`Upload failed: ${err}`, { title: "Error", kind: "error" })
    } finally {
      setUploading(false)
    }
  }

  const handleExportStems = async () => {
    if (!splitResult) return

    const dest = await open({
      title: "Choose export folder",
      directory: true,
      multiple: false,
    })
    if (!dest || typeof dest !== "string") return

    try {
      const exported = await invoke<string[]>("export_stems", {
        stems: splitResult.stems,
        destDir: dest,
      })
      await message(
        `Exported ${exported.length} stem(s) to:\n${dest}`,
        { title: "Export complete", kind: "info" }
      )
    } catch (err) {
      await message(`Export failed: ${err}`, { title: "Error", kind: "error" })
    }
  }

  // ── Render ───────────────────────────────────────────────────────────────────

  return (
    <div style={{ padding: 40, fontFamily: "sans-serif", maxWidth: 800 }}>
      <h1>Rustems</h1>

      {/* ── Device connection ── */}
      <section style={{ marginBottom: 24 }}>
        <h2 style={{ fontSize: 16, marginBottom: 8 }}>Device</h2>
        <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
          <select value={selectedDevice} onChange={(e) => setSelectedDevice(e.target.value)}>
            <option value="">-- Choose a device --</option>
            {devices.map((d) => <option key={d} value={d}>{d}</option>)}
          </select>

          {/* Manual refresh — always visible when not connected */}
          {!connected && (
            <button
              onClick={handleRefreshDevices}
              disabled={refreshing}
              title="Scan for devices"
              style={{ fontSize: 16, padding: "2px 10px", lineHeight: 1 }}
            >
              {refreshing ? "…" : "↻"}
            </button>
          )}

          <button onClick={handleConnect} disabled={!selectedDevice || connected}>Connect</button>

          {connected && (
            <button onClick={handleDisconnect} style={{ color: "#e05555" }}>Disconnect</button>
          )}
          {connected && (
            <button onClick={async () => { await fetchTracks(); await fetchStorage() }} disabled={loadingTracks}>
              {loadingTracks ? "Refreshing…" : "↻ Refresh tracks"}
            </button>
          )}

        </div>
        {connected && storage && <StorageBar storage={storage} />}
      </section>

      {/* ── Stem Splitter ── */}
      <section style={{
        marginBottom: 24,
        border: "1px solid #3a5a3a",
        borderRadius: 12,
        padding: 20,
        background: "linear-gradient(135deg, #0d1f0d 0%, #111a11 60%, #0a150a 100%)",
        boxShadow: "0 4px 24px rgba(0,0,0,0.5)",
      }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10 }}>
          <h2 style={{ fontSize: 18, margin: 0, fontFamily: "'Georgia', serif", color: "#7dda7d", letterSpacing: "0.04em", fontWeight: 700 }}>✂ Stem Splitter</h2>
          <SplitterBadge status={splitterStatus} detail={splitterDetail} />
        </div>

        {/* Setup instructions when missing */}
        {splitterStatus === "missing" && (
          <div style={{
            fontSize: 12, color: "#f99", marginBottom: 12,
            background: "#1f0d0d", borderRadius: 6, padding: "10px 14px",
            border: "1px solid #4a2020",
          }}>
            <strong>Model unavailable.</strong> To enable stem splitting:
            <ol style={{ margin: "6px 0 6px 16px", padding: 0, lineHeight: 1.8 }}>
              <li>No action needed — the splitter is built into the app</li>
              <li></li>
              <li>The model (~100 MB) will be downloaded automatically on first use</li>
            </ol>
            <button onClick={checkSplitter} style={{ fontSize: 11, padding: "3px 10px", cursor: "pointer" }}>
              Re-check
            </button>
          </div>
        )}

        <p style={{ fontSize: 12, color: "#666", margin: "0 0 12px" }}>
          Split any song into 4 stems locally.
          CPU splitting takes 1-3 min per track. GPU is used automatically if available.
        </p>

        <button
          onClick={handlePickAndSplit}
          disabled={splitting || splitterStatus !== "ok"}
        >
          {splitting ? "Splitting…" : "Pick file & split"}
        </button>

        {splitting && (
          <span style={{ marginLeft: 12, fontSize: 13, color: "orange" }}>
            <Spinner /> Splitting stems — this may take a minute…
          </span>
        )}

        {/* Split result card */}
        {splitResult && !splitting && (
          <div style={{
            marginTop: 14,
            border: "1px solid #2a4a2a",
            borderRadius: 6,
            padding: 14,
            background: "#0a180a",
          }}>
            <div style={{ fontSize: 13, color: "#5d5", marginBottom: 8, fontWeight: 600 }}>
              ✓ Split complete
            </div>

            {/* Preview */}
            <div style={{ marginBottom: 10 }}>
              <span style={{ fontSize: 12, color: "#666", marginRight: 8 }}>Preview:</span>
              <button onClick={play} style={{ marginRight: 6, fontSize: 12 }}>▶ Play</button>
              <button onClick={pause} style={{ fontSize: 12 }}>⏸ Pause</button>
            </div>
            <div style={{ display: "flex", gap: 40, marginBottom: 14 }}>
              <StemSlider stem="drums" />
              <StemSlider stem="bass" />
              <StemSlider stem="melody" />
              <StemSlider stem="vocals" />
            </div>

            {/* Track name + upload */}
            <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
              <label style={{ fontSize: 12, color: "#888" }}>Track name:</label>
              <input
                type="text"
                value={splitTrackName}
                onChange={(e) => setSplitTrackName(e.target.value)}
                style={{ padding: "4px 8px", width: 200, fontSize: 13 }}
              />
              <button
                onClick={handleUploadSplit}
                disabled={uploading || !connected}
                style={{
                  background: connected ? "#152515" : "#1a1a1a",
                  color: connected ? "#5d5" : "#555",
                  border: `1px solid ${connected ? "#2a4a2a" : "#333"}`,
                  cursor: connected ? "pointer" : "not-allowed",
                }}
              >
                {uploading ? "Uploading…" : "Upload to device"}
              </button>

              <button
                onClick={handleExportStems}
                style={{
                  background: "#151525",
                  color: "#88aaff",
                  border: "1px solid #2a2a4a",
                  cursor: "pointer",
                }}
              >
                Export to folder…
              </button>
              {!connected && (
                <span style={{ fontSize: 11, color: "#555" }}>Connect a device first</span>
              )}
            </div>
          </div>
        )}
      </section>

      {/* ── Manual stem preview & upload ── */}
      <section style={{ marginBottom: 32 }}>
        <h2 style={{ fontSize: 16, marginBottom: 4 }}>Manual Upload</h2>
        <p style={{ fontSize: 12, color: "#666", margin: "0 0 10px" }}>
          Pick a folder containing <code>melody.mp3</code>, <code>vocals.mp3</code>, <code>bass.mp3</code>, <code>drums.mp3</code>.
        </p>
        <button onClick={handlePickAndLoad}>Select Stem Folder</button>
        {folder && <p style={{ marginTop: 8, fontSize: 12, color: "#666" }}>Loaded: {folder}</p>}
        <div style={{ marginTop: 10 }}>
          <button onClick={play} style={{ marginRight: 8 }}>▶ Play</button>
          <button onClick={pause}>⏸ Pause</button>
        </div>
        <div style={{
          display: "flex", gap: 40, marginTop: 16,
          opacity: uploading ? 0.5 : 1,
          pointerEvents: uploading ? "none" : "auto",
        }}>
          <StemSlider stem="drums" />
          <StemSlider stem="bass" />
          <StemSlider stem="melody" />
          <StemSlider stem="vocals" />
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 14, marginBottom: 10 }}>
          <label style={{ fontSize: 13 }}>Track name:</label>
          <input
            type="text"
            value={trackName}
            onChange={(e) => setTrackName(e.target.value)}
            placeholder="e.g. My Song"
            style={{ padding: "4px 8px", width: 220 }}
          />
        </div>
        <button onClick={handleUpload} disabled={!folder || !selectedDevice || uploading}>
          {uploading ? "Uploading…" : "Upload Stems"}
        </button>
        {uploading && <span style={{ marginLeft: 12, color: "orange" }}>Please wait…</span>}
      </section>

      {/* ── Album / track browser ── */}
      {connected && (
        <section>
          <h2 style={{ fontSize: 16, marginBottom: 12 }}>On Device</h2>
          {loadingTracks && <p style={{ color: "#888" }}>Loading…</p>}
          {!loadingTracks && albums.length === 0 && (
            <p style={{ color: "#888", fontSize: 13 }}>No tracks found on device.</p>
          )}
          {albums.map((album) => (
            <div key={album.album_id} style={{
              border: "1px solid #333", borderRadius: 8, marginBottom: 16, overflow: "hidden",
            }}>
              <div style={{
                display: "flex", justifyContent: "space-between", alignItems: "center",
                padding: "8px 14px", background: "#1a1a1a",
              }}>
                <span style={{ fontWeight: 600, fontSize: 14 }}>{album.album_id}</span>
                <button
                  onClick={() => handleDeleteAlbum(album.album_id)}
                  style={{ color: "#e05555", background: "none", border: "none", cursor: "pointer", fontSize: 12 }}
                >
                  Delete album
                </button>
              </div>
              {album.tracks.map((track) => (
                <div key={track.track_id} style={{
                  display: "flex", alignItems: "center", justifyContent: "space-between",
                  padding: "8px 14px", borderTop: "1px solid #2a2a2a",
                }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                    <div style={{
                      width: 28, height: 28, borderRadius: 4, flexShrink: 0,
                      background: `linear-gradient(135deg, ${track.colours[0] ?? "#444"} 50%, ${track.colours[1] ?? "#888"} 50%)`,
                    }} />
                    <div>
                      <div style={{ fontWeight: 500, fontSize: 14 }}>{track.title}</div>
                      <div style={{ fontSize: 12, color: "#888" }}>
                        {track.artist} · {track.track_id}
                      </div>
                    </div>
                  </div>
                  <button
                    onClick={() => handleDeleteTrack(track.album_id, track.track_id, track.title)}
                    style={{ color: "#e05555", background: "none", border: "none", cursor: "pointer", fontSize: 12 }}
                  >
                    Delete
                  </button>
                </div>
              ))}
            </div>
          ))}
        </section>
      )}
    </div>
  )
}

// ── Sub-components ────────────────────────────────────────────────────────────

function SplitterBadge({
  status
}: { status: "unknown" | "checking" | "ok" | "missing"; detail: string }) {
  const cfg = {
    unknown:  { bg: "#1a1a1a", color: "#555",  border: "#333",   dot: "●" },
    checking: { bg: "#1a1a1a", color: "#888",  border: "#333",   dot: "○" },
    ok:       { bg: "#0d1f0d", color: "#5d5",  border: "#1a3a1a", dot: "●" },
    missing:  { bg: "#1f0d0d", color: "#e77",  border: "#3a1a1a", dot: "●" },
  }[status]

  const label = {
    unknown:  "Splitter: unknown",
    checking: "Checking…",
    ok:       "Ready",
    missing:  "Splitter not found",
  }[status]

  return (
    <span style={{
      fontSize: 11, padding: "2px 8px", borderRadius: 99,
      background: cfg.bg, color: cfg.color,
      border: `1px solid ${cfg.border}`,
    }}>
      {cfg.dot} {label}
    </span>
  )
}

function Spinner() {
  return (
    <>
      <style>{`@keyframes rs-spin { to { transform: rotate(360deg); } }`}</style>
      <span style={{
        display: "inline-block", width: 11, height: 11,
        border: "2px solid #444", borderTop: "2px solid orange",
        borderRadius: "50%", animation: "rs-spin 0.7s linear infinite",
        marginRight: 6, verticalAlign: "middle",
      }} />
    </>
  )
}