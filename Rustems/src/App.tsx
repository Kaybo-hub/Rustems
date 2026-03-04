import StemSlider from "./components/StemSlider"
import { useStemPlayer } from "./hooks/useStemPlayer"
import { useEffect, useState } from "react"
import { pickFolder } from "./components/StemSlider"
import { invoke } from "@tauri-apps/api/core"
import { message } from "@tauri-apps/plugin-dialog"

export default function App() {
  const { loadSong, play, pause } = useStemPlayer()
  const [folder, setFolder] = useState<string | null>(null)
  const [devices, setDevices] = useState<string[]>([])
  const [selectedDevice, setSelectedDevice] = useState("")
  const [uploading, setUploading] = useState(false)

  useEffect(() => {
    invoke<string[]>("list_usb_devices").then(setDevices)
  }, [])

  const handlePickAndLoad = async () => {
    const selectedFolder = await pickFolder()
    if (!selectedFolder) return

    setFolder(selectedFolder)
    await loadSong(selectedFolder)
  }

  const handleConnect = async () => {
    if (!selectedDevice) return
    try {
      await invoke<string>("connect_usb_device", { serial: selectedDevice })
    } catch (err) {
      await message(`Failed to connect: ${err}`, { title: "Error", kind: "error" })
    }
  }

  const handleUpload = async () => {
    if (!folder) {
      await message("Load a folder first", { title: "Rustems", kind: "warning" })
      return
    }
    setUploading(true)
    try {
      await invoke("upload_stems", { folder })
      await message("Upload complete!", { title: "Rustems", kind: "info" })
    } catch (err) {
      await message(`Upload failed: ${err}`, { title: "Error", kind: "error" })
    } finally {
      setUploading(false)
    }
  }

  return (
    <div style={{ padding: 40 }}>
      <h1>Rustems</h1>

      <button onClick={handlePickAndLoad}>
        Select Stem Folder
      </button>

      {folder && (
        <p style={{ marginTop: 10 }}>
          Loaded: {folder}
        </p>
      )}

      <div style={{ marginTop: 20 }}>
        <button onClick={play}>Play</button>
        <button onClick={pause}>Pause</button>
      </div>

      <button onClick={async () => {
          const result = await invoke<string>("check_device_state")
          await message(result, { title: "Device State" })
      }}>
          Check State
      </button>

      <div style={{ marginTop: 20 }}>
        <label>Select USB Device:</label>
        <select
          value={selectedDevice}
          onChange={(e) => setSelectedDevice(e.target.value)}
        >
          <option value="">-- Choose a device --</option>
          {devices.map((d) => (
            <option key={d} value={d}>
              {d}
            </option>
          ))}
        </select>
        <button onClick={handleConnect}>Connect</button>
      </div>

      <button onClick={handleUpload} disabled={!folder || !selectedDevice}>
        Upload to Device
      </button>

      {uploading && <p style={{ color: "orange" }}>Uploading stems, please wait...</p>}

      <div style={{ display: "flex", gap: 40, marginTop: 40, opacity: uploading ? 0.5 : 1, pointerEvents: uploading ? "none" : "auto" }}>
        <StemSlider stem="drums" />
        <StemSlider stem="bass" />
        <StemSlider stem="melody" />
        <StemSlider stem="vocals" />
      </div>
    </div>
  )
}