import { useState } from "react"
import { useStemPlayer } from "../hooks/useStemPlayer"

interface Props {
  stem: string
}

export default function StemSlider({ stem }: Props) {
  const { setStemVolume } = useStemPlayer()
  const [volume, setVolume] = useState(1)

  const handleChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const value = parseFloat(e.target.value)
    setVolume(value)
    await setStemVolume(stem, value)
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
        style={{ writingMode: "vertical-lr", transform: "rotate(180deg)", height: "150px" }}
      />
      <span>{stem.toUpperCase()}</span>
    </div>
  )
}