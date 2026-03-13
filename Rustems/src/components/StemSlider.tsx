import { useState } from "react"
import { useStemPlayer } from "../hooks/useStemPlayer"
import { open } from "@tauri-apps/plugin-dialog";

interface Props {
  stem: string;
  onValueChange?: (value: number) => void; 
}

export default function StemSlider({ stem, onValueChange }: Props) {
  const { setStemVolume } = useStemPlayer()
  const [volume, setVolume] = useState(1)

  const handleChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const value = parseFloat(e.target.value)
    setVolume(value)
    
    await setStemVolume(stem, value)

    if (onValueChange) {
      // Map 0.0-1.0 to 0-255 for the hardware LEDs
      onValueChange(Math.round(value * 255));
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center" }}>
      <input
        type="range"
        min="0"
        max="1"
        step="0.01"
        value={volume}
        onChange={handleChange}
        style={{ 
          writingMode: "vertical-lr", 
          direction: "rtl",
          verticalAlign: "middle",
          height: "150px",
          cursor: "pointer",
        }}
      />
      <span style={{ marginTop: "10px", fontSize: "12px", fontWeight: "bold", color: "#aaa" }}>
        {stem.toUpperCase()}
      </span>
    </div>
  )
}

export async function pickFolder(): Promise<string | null> {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "Select Stem Folder",
  });

  if (!selected) return null;
  return Array.isArray(selected) ? selected[0] : selected;
}