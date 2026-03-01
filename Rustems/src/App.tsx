import StemSlider from "./components/StemSlider"
import { useStemPlayer } from "./hooks/useStemPlayer"
import { useState } from "react"
import { pickFolder } from "./components/StemSlider"

export default function App() {
  const { loadSong, play, pause } = useStemPlayer()
  const [folder, setFolder] = useState<string | null>(null)

  const handlePickAndLoad = async () => {
    const selectedFolder = await pickFolder()
    if (!selectedFolder) return

    setFolder(selectedFolder)
    await loadSong(selectedFolder)
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

      <div style={{ display: "flex", gap: 40, marginTop: 40 }}>
        <StemSlider stem="drums" />
        <StemSlider stem="bass" />
        <StemSlider stem="melody" />
        <StemSlider stem="vocals" />
      </div>
    </div>
  )
}