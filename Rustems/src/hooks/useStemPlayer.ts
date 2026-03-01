import {
  invoke
} from "@tauri-apps/api/core"

export function useStemPlayer() {

  const loadSong = async (folder: string) => {
    console.log("Loading stems from:", folder)
    await invoke("load_song", {
      folder
    })
  }

  const play = async () => {
    await invoke("play")
  }

  const pause = async () => {
    await invoke("pause")
  }

  const setStemVolume = async (stem: string, volume: number) => {
    try {
      await invoke("set_stem_volume", {
        stem,
        volume
      });
    } catch (error) {
      console.error("Failed to set volume:", error)
    }
  }

  const printStatus = async () => {
    try {
      await invoke("print_audio_status");
    } catch (error) {
      console.error("Failed to print status:", error);
    }
  }


  return {
    loadSong,
    play,
    pause,
    setStemVolume,
    printStatus
  }
}