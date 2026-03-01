import StemSlider from "./components/StemSlider"
import { useStemPlayer } from "./hooks/useStemPlayer"
import { useState } from "react"

export default function App() {
  const { loadSong, play, pause } = useStemPlayer()
  const [folder, setFolder] = useState("")

  return (
    <div style={{ padding: 40 }}>
      <h1>Rustems</h1>

      <input
        placeholder="Path to song folder"
        value={folder}
        onChange={(e) => setFolder(e.target.value)}
        style={{ width: 300 }}
      />

      <button onClick={() => loadSong(folder)}>Load Song</button>
      <button onClick={play}>Play</button>
      <button onClick={pause}>Pause</button>

      <div style={{ display: "flex", gap: 40, marginTop: 40 }}>
        <StemSlider stem="drums" />
        <StemSlider stem="bass" />
        <StemSlider stem="melody" />
        <StemSlider stem="vocals" />
      </div>
    </div>
  )
}