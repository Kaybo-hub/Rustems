import StemSlider from "./components/StemSlider"
import { useStemPlayer } from "./hooks/useStemPlayer"
import { useEffect, useState } from "react"
import { pickFolder } from "./components/StemSlider"
import { invoke } from "@tauri-apps/api/core"
import { useThrottle } from "./hooks/useThrottle"

export default function App() {
  const { loadSong, play, pause } = useStemPlayer()
  const [folder, setFolder] = useState<string | null>(null)
  const [devices, setDevices] = useState<string[]>([])
  const [selectedDevice, setSelectedDevice] = useState("")
  const [vibe, setVibe] = useState({ r: 255, g: 255, b: 255 });

  useEffect(() => {
    invoke<string[]>("list_usb_devices").then(setDevices)
  }, [])

  const throttledLedUpdate = useThrottle((r: number, g: number, b: number) => {
    if (selectedDevice) {
      invoke("set_led_color", {
        serial: selectedDevice,
        red: r,
        green: g,
        blue: b
      }).catch(console.error);
    }
  }, 50);

  useEffect(() => {
    throttledLedUpdate(vibe.r, vibe.g, vibe.b);
  }, [vibe, throttledLedUpdate])

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
      console.error("Failed to connect:", err)
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

      <div style={{ display: "flex", gap: 40, marginTop: 40 }}>
        <StemSlider
          stem="drums"
          onValueChange={(val) => setVibe(prev => ({...prev, r: val}))}
        />
        <StemSlider
          stem="bass"
          onValueChange={(val) => setVibe(prev => ({...prev, g: val}))}
        />
        <StemSlider
          stem="melody"
          onValueChange={(val) => setVibe(prev => ({...prev, b: val}))}
        />
        <StemSlider
          stem="vocals"
          onValueChange={(val) => setVibe({r: val, g: val, b: val})}
        />
      </div>
    </div>
  )
}