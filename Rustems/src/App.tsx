import StemSlider from "./components/StemSlider"
import { useStemPlayer, listDeviceTracks, deleteTrack, deleteAlbum } from "./hooks/useStemPlayer"
import type { AlbumInfo } from "./hooks/useStemPlayer"
import { useEffect, useState } from "react"
import { pickFolder } from "./components/StemSlider"
import { invoke } from "@tauri-apps/api/core"
import { message, confirm } from "@tauri-apps/plugin-dialog"

interface StorageInfo {
  total_bytes: number
  free_bytes: number
  used_bytes: number
}

function StorageBar({ storage }: { storage: StorageInfo }) {
  const usedPct = storage.total_bytes > 0
    ? Math.min(100, (storage.used_bytes / storage.total_bytes) * 100)
    : 0
  const freeMB  = (storage.free_bytes * 1024 / 1_073_741_824).toFixed(2)
  const totalMB = (storage.total_bytes * 1024 / 1_073_741_824).toFixed(2)
  const usedMB  = (storage.used_bytes * 1024 / 1_073_741_824).toFixed(2)

  const barColour = usedPct > 90 ? "#e05555" : usedPct > 70 ? "#e08c30" : "#4caf82"

  return (
    <div style={{ marginTop: 12, marginBottom: 4 }}>
      <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, color: "#888", marginBottom: 5 }}>
        <span>Storage</span>
        <span>{usedMB} GB used · {freeMB} GB free · {totalMB} GB total</span>
      </div>
      <div style={{
        height: 6, borderRadius: 3, background: "#2a2a2a", overflow: "hidden"
      }}>
        <div style={{
          height: "100%", width: `${usedPct}%`,
          background: barColour,
          borderRadius: 3,
          transition: "width 0.4s ease"
        }} />
      </div>
    </div>
  )
}

export default function App() {
  const { loadSong, play, pause } = useStemPlayer()
  const [folder, setFolder] = useState<string | null>(null)
  const [trackName, setTrackName] = useState("")
  const [devices, setDevices] = useState<string[]>([])
  const [selectedDevice, setSelectedDevice] = useState("")
  const [uploading, setUploading] = useState(false)
  const [albums, setAlbums] = useState<AlbumInfo[]>([])
  const [loadingTracks, setLoadingTracks] = useState(false)
  const [connected, setConnected] = useState(false)
  const [storage, setStorage] = useState<StorageInfo | null>(null)
  const [stemSizes, setStemSizes] = useState<number>(0)

  useEffect(() => {
    invoke<string[]>("list_usb_devices").then(setDevices)
  }, [])

  // Recompute estimated stem sizes whenever a folder is loaded
  useEffect(() => {
    if (!folder) { setStemSizes(0); return }
    // Ask Tauri for file sizes of the 4 stems
    Promise.all(
      ["melody", "vocals", "bass", "drums"].map(name =>
        invoke<number>("get_file_size", { path: `${folder}/${name}.mp3` }).catch(() => 0)
      )
    ).then(sizes => setStemSizes((sizes as number[]).reduce((a, b) => a + b, 0)))
  }, [folder])

  const fetchStorage = async () => {
    try {
      const s = await invoke<StorageInfo>("get_storage_info")
      setStorage(s)
    } catch {
      // non-fatal — storage bar just won't show
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

  const handlePickAndLoad = async () => {
    const selectedFolder = await pickFolder()
    if (!selectedFolder) return
    setFolder(selectedFolder)
    const parts = selectedFolder.replace(/\\/g, "/").split("/")
    setTrackName(parts[parts.length - 1] ?? "")
    await loadSong(selectedFolder)
  }

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

  const notEnoughSpace = storage !== null && stemSizes > 0 &&
    Math.round(stemSizes * 1.1) > storage.free_bytes * 1024

  const handleUpload = async () => {
    if (!folder) {
      await message("Load a folder first", { title: "Rustems", kind: "warning" })
      return
    }
    if (notEnoughSpace) {
      await message(
        `Not enough storage on device. Need ~${(stemSizes * 1.1 / 1_073_741_824).toFixed(2)} GB, only ${(storage!.free_bytes / 1_073_741_824).toFixed(2)} GB free.`,
        { title: "Not Enough Space", kind: "error" }
      )
      return
    }
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

  const handleDeleteTrack = async (albumId: string, trackId: string, title: string) => {
    const yes = await confirm(`Delete "${title}" from the device? This cannot be undone.`, {
      title: "Delete Track",
      kind: "warning",
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

  return (
    <div style={{ padding: 40, fontFamily: "sans-serif", maxWidth: 800 }}>
      <h1>Rustems</h1>

      {/* ── Device connection ── */}
      <section style={{ marginBottom: 24 }}>
        <h2 style={{ fontSize: 16, marginBottom: 8 }}>Device</h2>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <select
            value={selectedDevice}
            onChange={(e) => setSelectedDevice(e.target.value)}
          >
            <option value="">-- Choose a device --</option>
            {devices.map((d) => (
              <option key={d} value={d}>{d}</option>
            ))}
          </select>
          <button onClick={handleConnect} disabled={!selectedDevice || connected}>Connect</button>
          {connected && (
            <button onClick={handleDisconnect} style={{ color: "#e05555" }}>Disconnect</button>
          )}
          {connected && (
            <button onClick={async () => { await fetchTracks(); await fetchStorage() }} disabled={loadingTracks}>
              {loadingTracks ? "Refreshing…" : "↻ Refresh"}
            </button>
          )}
        </div>
        {connected && storage && <StorageBar storage={storage} />}
      </section>

      {/* ── Stem player ── */}
      <section style={{ marginBottom: 24 }}>
        <h2 style={{ fontSize: 16, marginBottom: 8 }}>Preview</h2>
        <button onClick={handlePickAndLoad}>Select Stem Folder</button>
        {folder && <p style={{ marginTop: 8, fontSize: 13, color: "#888" }}>Loaded: {folder}</p>}
        <div style={{ marginTop: 10 }}>
          <button onClick={play}>Play</button>
          <button onClick={pause} style={{ marginLeft: 8 }}>Pause</button>
        </div>
        <div style={{
          display: "flex", gap: 40, marginTop: 20,
          opacity: uploading ? 0.5 : 1,
          pointerEvents: uploading ? "none" : "auto"
        }}>
          <StemSlider stem="drums" />
          <StemSlider stem="bass" />
          <StemSlider stem="melody" />
          <StemSlider stem="vocals" />
        </div>
      </section>

      {/* ── Upload ── */}
      <section style={{ marginBottom: 32 }}>
        <h2 style={{ fontSize: 16, marginBottom: 8 }}>Upload to Device</h2>
        <div style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 10 }}>
          <label style={{ fontSize: 13 }}>Track name:</label>
          <input
            type="text"
            value={trackName}
            onChange={(e) => setTrackName(e.target.value)}
            placeholder="e.g. My Song"
            style={{ padding: "4px 8px", width: 220 }}
          />
        </div>

        {/* Space warning */}
        {notEnoughSpace && (
          <p style={{ fontSize: 12, color: "#e05555", marginBottom: 8 }}>
            ⚠ Not enough space — need ~{(stemSizes * 1.1 * 1024 / 1_073_741_824).toFixed(2)} GB,
            only {(storage!.free_bytes / 1_073_741_824).toFixed(2)} GB free.
          </p>
        )}

        <button
          onClick={handleUpload}
          disabled={!folder || !selectedDevice || uploading || notEnoughSpace}
          style={notEnoughSpace ? { opacity: 0.4, cursor: "not-allowed" } : {}}
        >
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
              border: "1px solid #333",
              borderRadius: 8,
              marginBottom: 16,
              overflow: "hidden"
            }}>
              <div style={{
                display: "flex", justifyContent: "space-between", alignItems: "center",
                padding: "8px 14px", background: "#1a1a1a"
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
                  padding: "8px 14px", borderTop: "1px solid #2a2a2a"
                }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                    <div style={{
                      width: 28, height: 28, borderRadius: 4, flexShrink: 0,
                      background: `linear-gradient(135deg, ${track.colours[0] ?? "#444"} 50%, ${track.colours[1] ?? "#888"} 50%)`
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