import { invoke } from "@tauri-apps/api/core"

export interface TrackInfo {
  album_id: string
  track_id: string
  title: string
  artist: string
  colours: string[]
}

export interface AlbumInfo {
  album_id: string
  tracks: TrackInfo[]
}

export function useStemPlayer() {

  const loadSong = async (folder: string) => {
    console.log("Loading stems from:", folder)
    await invoke("load_song", { folder })
  }

  const play = async () => {
    await invoke("play")
  }

  const pause = async () => {
    await invoke("pause")
  }

  const setStemVolume = async (stem: string, volume: number) => {
    try {
      await invoke("set_stem_volume", { stem, volume })
    } catch (error) {
      console.error("Failed to set volume:", error)
    }
  }

  const printStatus = async () => {
    try {
      await invoke("print_audio_status")
    } catch (error) {
      console.error("Failed to print status:", error)
    }
  }

  return { loadSong, play, pause, setStemVolume, printStatus }
}

export async function getUSBDevices(): Promise<string[]> {
  return await invoke<string[]>("list_usb_devices")
}

export async function listDeviceTracks(): Promise<AlbumInfo[]> {
  return await invoke<AlbumInfo[]>("list_device_tracks")
}

export async function deleteTrack(albumId: string, trackId: string): Promise<void> {
  await invoke("delete_track", { albumId, trackId })
}

export async function deleteAlbum(albumId: string): Promise<void> {
  await invoke("delete_album", { albumId })
}