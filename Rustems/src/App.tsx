import StemSlider from "./components/StemSlider.js";
import {
  useStemPlayer,
  listDeviceTracks,
  deleteTrack,
  //deleteAlbum,
} from "./hooks/useStemPlayer.js";
import type { AlbumInfo, TrackInfo } from "./hooks/useStemPlayer.js";
import { useEffect, useRef, useState } from "react";
import { pickFolder } from "./components/StemSlider.js";
import { invoke } from "@tauri-apps/api/core";
import { open, message, confirm } from "@tauri-apps/plugin-dialog";
import "./App.css";

interface StorageInfo {
  total_bytes: number;
  free_bytes: number;
  used_bytes: number;
}

interface SplitResult {
  output_dir: string;
  track_name: string;
  stems: string[]; // [vocals.mp3, drums.mp3, bass.mp3, other.mp3]
}

document.addEventListener("contextmenu", (e) => e.preventDefault(), {
  capture: true,
});

document.addEventListener("keydown", (e) => {
  if (e.key === "F12") e.preventDefault();
  if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key === "I")
    e.preventDefault();
  if ((e.ctrlKey || e.metaKey) && e.key === "u") e.preventDefault();
});

function StorageBar({ storage }: { storage: StorageInfo }) {
  const usedPct =
    storage.total_bytes > 0
      ? Math.min(100, (storage.used_bytes / storage.total_bytes) * 100)
      : 0;
  // Device reports kilobytes - multiply by 1024 to get bytes, then convert to GB
  const toGB = (kb: number) => ((kb * 1024) / 1_073_741_824).toFixed(2);
  const barColour =
    usedPct > 90 ? "#ff2c2c" : usedPct > 70 ? "#e08c30" : "#32a850";

  return (
    <div style={{ marginTop: 12, marginBottom: 4 }}>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          fontSize: 11,
          color: "#888",
          marginBottom: 5,
        }}
      >
        <span>Storage</span>
        <span>
          {toGB(storage.used_bytes)} GB used · {toGB(storage.free_bytes)} GB
          free · {toGB(storage.total_bytes)} GB total
        </span>
      </div>
      <div
        style={{
          height: 6,
          borderRadius: 3,
          background: "#2a2a2a",
          overflow: "hidden",
        }}
      >
        <div
          style={{
            height: "100%",
            width: `${usedPct}%`,
            background: barColour,
            borderRadius: 3,
            transition: "width 0.4s ease",
          }}
        />
      </div>
    </div>
  );
}

export default function App() {
  const { loadSong, play, pause } = useStemPlayer();

  const [devices, setDevices] = useState<string[]>([]);
  const [selectedDevice, setSelectedDevice] = useState("");
  const [connected, setConnected] = useState(false);
  const [albums, setAlbums] = useState<AlbumInfo[]>([]);
  const [loadingTracks, setLoadingTracks] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [storage, setStorage] = useState<StorageInfo | null>(null);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [newAlbum, setNewAlbum] = useState(false);

  // Refs so the polling closure always sees current values without re-registering
  const connectedRef = useRef(connected);
  const selectedDeviceRef = useRef(selectedDevice);
  useEffect(() => {
    connectedRef.current = connected;
  }, [connected]);
  useEffect(() => {
    selectedDeviceRef.current = selectedDevice;
  }, [selectedDevice]);

  const [folder, setFolder] = useState<string | null>(null);
  const [trackName, setTrackName] = useState("");
  const [uploading, setUploading] = useState(false);

  const [splitterStatus, setSplitterStatus] = useState<
    "unknown" | "checking" | "ok" | "needs-download" | "downloading" | "missing"
  >("unknown");
  const [splitterDetail, setSplitterDetail] = useState("");
  const [splitting, setSplitting] = useState(false);
  const [splitResult, setSplitResult] = useState<SplitResult | null>(null);
  const [splitTrackName, setSplitTrackName] = useState("");

  // Called whenever the connected device has vanished
  const handleForcedDisconnect = () => {
    setConnected(false);
    setAlbums([]);
    setStorage(null);
    setSelectedDevice("");
    invoke("disconnect_device").catch(() => {});
  };

  useEffect(() => {
    invoke<string[]>("list_usb_devices").then(setDevices);
    checkSplitter();

    // Poll every 2s to track device presence.
    // While connected only check whether a device is still present;
    // while disconnected do the full add/remove bookkeeping.
    pollRef.current = setInterval(async () => {
      try {
        const found = await invoke<string[]>("list_usb_devices");

        if (connectedRef.current) {
          // Device is connected - check if it was physically unplugged
          if (!found.includes(selectedDeviceRef.current)) {
            handleForcedDisconnect();
          }
          return;
        }

        // Not connected - normal device list bookkeeping
        setDevices((prev) => {
          const added = found.filter((d) => !prev.includes(d));
          const removed = prev.filter((d) => !found.includes(d));
          if (added.length === 0 && removed.length === 0) return prev;
          if (added.length === 1) {
            setSelectedDevice((sel) => (sel === "" ? added[0] : sel));
          }
          if (removed.length > 0) {
            setSelectedDevice((sel) => (removed.includes(sel) ? "" : sel));
          }
          return found;
        });
      } catch {
        /* silently ignore scan errors */
      }
    }, 2000);

    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, []);

  const handleRefreshDevices = async () => {
    setRefreshing(true);
    try {
      const found = await invoke<string[]>("list_usb_devices");
      setDevices(found);
      if (found.length === 1) setSelectedDevice(found[0]);
    } finally {
      setRefreshing(false);
    }
  };

  const checkSplitter = async () => {
    setSplitterStatus("checking");
    try {
      const detail = await invoke<string>("check_splitter");
      setSplitterDetail(detail);
      setSplitterStatus("ok");
    } catch {
      // Not cached yet - needs a one-time download
      setSplitterStatus("needs-download");
    }
  };

  const handleDownloadModel = async () => {
    setSplitterStatus("downloading");
    try {
      const detail = await invoke<string>("download_model");
      setSplitterDetail(detail);
      setSplitterStatus("ok");
    } catch (err) {
      setSplitterDetail(String(err));
      setSplitterStatus("missing");
    }
  };

  const fetchStorage = async () => {
    try {
      const s = await invoke<StorageInfo>("get_storage_info");
      setStorage(s);
    } catch {
      /* non-fatal */
    }
  };

  const handleConnect = async () => {
    if (!selectedDevice) return;
    try {
      await invoke<string>("connect_usb_device", { serial: selectedDevice });
      setConnected(true);
      await fetchTracks();
      await fetchStorage();
    } catch (err) {
      await message(`Failed to connect: ${err}`, {
        title: "Error",
        kind: "error",
      });
    }
  };

  const handleDisconnect = async () => {
    try {
      await invoke("disconnect_device");
      setConnected(false);
      setAlbums([]);
      setStorage(null);
    } catch (err) {
      await message(`Disconnect failed: ${err}`, {
        title: "Error",
        kind: "error",
      });
    }
  };

  const fetchTracks = async () => {
    setLoadingTracks(true);
    try {
      const result = await listDeviceTracks();
      setAlbums(result);
    } catch (err) {
      await message(`Failed to list tracks: ${err}`, {
        title: "Error",
        kind: "error",
      });
    } finally {
      setLoadingTracks(false);
    }
  };

  const handlePickAndLoad = async () => {
    const selectedFolder = await pickFolder();
    if (!selectedFolder) return;
    setFolder(selectedFolder);
    const parts = selectedFolder.replace(/\\/g, "/").split("/");
    setTrackName(parts[parts.length - 1] ?? "");
    await loadSong(selectedFolder);
  };

  const handleUpload = async () => {
    if (!folder) {
      await message("Load a folder first", {
        title: "Rustems",
        kind: "warning",
      });
      return;
    }
    if (!connected) {
      await message("Connect a device first", {
        title: "Rustems",
        kind: "warning",
      });
      return;
    }
    const yes = await confirm(`Upload "${trackName}" to device?`, {
      title: "Confirm Upload",
      kind: "info",
    });
    if (!yes) return;

    setUploading(true);
    try {
      await invoke("upload_stems", {
        folder,
        trackName: trackName.trim() || undefined,
        newAlbum,
      });
      await message("Upload complete!", { title: "Rustems", kind: "info" });
      await fetchTracks();
      await fetchStorage();
    } catch (err) {
      await message(`Upload failed: ${err}`, { title: "Error", kind: "error" });
    } finally {
      setUploading(false);
    }
  };

  const handleDeleteTrack = async (
    albumId: string,
    trackId: string,
    title: string
  ) => {
    const yes = await confirm(
      `Delete "${title}" from the device? This cannot be undone.`,
      {
        title: "Delete Track",
        kind: "warning",
      }
    );
    if (!yes) return;
    try {
      await deleteTrack(albumId, trackId);
      await fetchTracks();
      await fetchStorage();
    } catch (err) {
      await message(`Delete failed: ${err}`, { title: "Error", kind: "error" });
    }
  };
  /*
  const handleDeleteAlbum = async (albumId: string) => {
    const yes = await confirm(
      `Delete entire album ${albumId} and all its tracks? This cannot be undone.`,
      { title: "Delete Album", kind: "warning" }
    );
    if (!yes) return;
    try {
      await deleteAlbum(albumId);
      await fetchTracks();
      await fetchStorage();
    } catch (err) {
      await message(`Delete failed: ${err}`, { title: "Error", kind: "error" });
    }
  };
  */

  const handlePickAndSplit = async () => {
    const selected = await open({
      title: "Select audio file to split",
      multiple: false,
      filters: [
        {
          name: "Audio",
          extensions: ["mp3", "wav", "flac", "aac", "ogg", "m4a"],
        },
      ],
    });
    if (!selected || typeof selected !== "string") return;

    setSplitting(true);
    setSplitResult(null);
    try {
      const result = await invoke<SplitResult>("split_stems", {
        inputPath: selected,
      });
      setSplitResult(result);
      setSplitTrackName(result.track_name);
      await loadSong(result.output_dir);
    } catch (err) {
      await message(`Splitting failed: ${err}`, {
        title: "Splitter Error",
        kind: "error",
      });
    } finally {
      setSplitting(false);
    }
  };

  const handleUploadSplit = async () => {
    if (!splitResult) return;
    if (!connected) {
      await message("Connect a device first", {
        title: "Rustems",
        kind: "warning",
      });
      return;
    }
    const yes = await confirm(
      `Upload "${splitTrackName}" to device?\n\nStems: vocals, drums, bass, melody`,
      { title: "Confirm Upload", kind: "info" }
    );
    if (!yes) return;

    setUploading(true);
    try {
      await invoke("upload_stems", {
        folder: splitResult.output_dir,
        trackName: splitTrackName.trim() || splitResult.track_name,
        newAlbum,
      });
      await message("Upload complete!", { title: "Rustems", kind: "info" });
      setSplitResult(null);
      await fetchTracks();
      await fetchStorage();
    } catch (err) {
      await message(`Upload failed: ${err}`, { title: "Error", kind: "error" });
    } finally {
      setUploading(false);
    }
  };

  const handleExportStems = async () => {
    if (!splitResult) return;

    const dest = await open({
      title: "Choose export folder",
      directory: true,
      multiple: false,
    });
    if (!dest || typeof dest !== "string") return;

    try {
      const exported = await invoke<string[]>("export_stems", {
        stems: splitResult.stems,
        destDir: dest,
      });
      await message(`Exported ${exported.length} stem(s) to:\n${dest}`, {
        title: "Export complete",
        kind: "info",
      });
    } catch (err) {
      await message(`Export failed: ${err}`, { title: "Error", kind: "error" });
    }
  };

  return (
    <div style={{ padding: 40, fontFamily: "sans-serif", maxWidth: 800, margin: "0 auto" }}>
      <h1>Rustems</h1>

      {/* Device connection */}
      <section style={{ marginBottom: 24 }}>
        <h2 style={{ fontSize: 16, marginBottom: 8 }}>Device</h2>
        <div
          style={{
            display: "flex",
            gap: 8,
            alignItems: "center",
            flexWrap: "wrap",
          }}
        >
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

          {/* Manual refresh - always visible when not connected */}
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

          <button
            onClick={handleConnect}
            disabled={!selectedDevice || connected}
          >
            Connect
          </button>

          {connected && (
            <button onClick={handleDisconnect} style={{ color: "#e05555" }}>
              Disconnect
            </button>
          )}
          {connected && (
            <button
              onClick={async () => {
                await fetchTracks();
                await fetchStorage();
              }}
              disabled={loadingTracks}
            >
              {loadingTracks ? "Refreshing…" : "↻ Refresh tracks"}
            </button>
          )}
        </div>
        {connected && storage && <StorageBar storage={storage} />}
      </section>

      {/* Stem Splitter */}
      <section
        style={{
          marginBottom: 24,
          border: "1px solid #333",
          borderRadius: 12,
          padding: 20,
          background: "#1a1a1a",
          boxShadow: "0 4px 24px rgba(0,0,0,0.5)",
        }}
      >
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            marginBottom: 10,
          }}
        >
          <h2
            style={{
              fontSize: 18,
              margin: 0,
              fontFamily: "sans-serif",
              color: "#ffffff",
              letterSpacing: "0.04em",
              fontWeight: 700,
            }}
          >
            Stem Splitter
          </h2>
          <SplitterBadge status={splitterStatus} detail={splitterDetail} />
        </div>

        {/* Setup instructions when missing */}
        {splitterStatus === "needs-download" && (
          <div style={{
            fontSize: 12, color: "orange", marginBottom: 12,
            background: "#1f180d", borderRadius: 6, padding: "10px 14px",
            border: "1px solid #4a3820",
          }}>
            <strong>One-time setup:</strong> The AI model (~100 MB) needs to be
            downloaded before splitting works. This only happens once.
            <div style={{ marginTop: 8 }}>
              <button onClick={handleDownloadModel} style={{ fontSize: 12 }}>
                Download model
              </button>
            </div>
          </div>
        )}

        {splitterStatus === "downloading" && (
          <div style={{ fontSize: 12, color: "orange", marginBottom: 12 }}>
            <Spinner /> Downloading model (~100 MB) - please stay connected…
          </div>
        )}

        {splitterStatus === "missing" && (
          <div style={{
            fontSize: 12, color: "#f99", marginBottom: 12,
            background: "#1f0d0d", borderRadius: 6, padding: "10px 14px",
            border: "1px solid #4a2020",
          }}>
            <strong>Download failed.</strong> Check your connection and try again.
            <div style={{ marginTop: 8 }}>
              <button onClick={handleDownloadModel} style={{ fontSize: 12 }}>
                Retry download
              </button>
            </div>
          </div>
        )}

        <p style={{ fontSize: 12, color: "#666", margin: "0 0 12px" }}>
          Split any song into 4 stems locally. CPU splitting takes 1-3 min per
          track. GPU is used automatically if available.
        </p>

        <button
          onClick={handlePickAndSplit}
          disabled={splitting || splitterStatus !== "ok"}
        >
          {splitting ? "Splitting…" : "Pick file & split"}
        </button>

        {splitting && (
          <span style={{ marginLeft: 12, fontSize: 13, color: "orange" }}>
            <Spinner /> Splitting stems - this may take a minute…
          </span>
        )}

        {/* Split result card */}
        {splitResult && !splitting && (
          <div
            style={{
              marginTop: 14,
              border: "1px solid #333",
              borderRadius: 6,
              padding: 14,
              background: "#1a1a1a",
            }}
          >
            <div
              style={{
                fontSize: 13,
                color: "#ccc",
                marginBottom: 8,
                fontWeight: 600,
              }}
            >
              Split complete
            </div>

            {/* Preview */}
            <div style={{ marginBottom: 10 }}>
              <span style={{ fontSize: 12, color: "#666", marginRight: 8 }}>
                Preview:
              </span>
              <button onClick={play} style={{ marginRight: 6, fontSize: 12 }}>
                ▶ Play
              </button>
              <button onClick={pause} style={{ fontSize: 12 }}>
                ⏸ Pause
              </button>
            </div>
            <div style={{ display: "flex", gap: 40, marginBottom: 14 }}>
              <StemSlider stem="drums" />
              <StemSlider stem="bass" />
              <StemSlider stem="melody" />
              <StemSlider stem="vocals" />
            </div>

            {/* Track name + upload */}
            <div
              style={{
                display: "flex",
                gap: 8,
                alignItems: "center",
                flexWrap: "wrap",
              }}
            >
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
                  background: connected ? "#222" : "#1a1a1a",
                  color: connected ? "#eee" : "#555",
                  border: `1px solid ${connected ? "#444" : "#333"}`,
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
                <span style={{ fontSize: 11, color: "#555" }}>
                  Connect a device first
                </span>
              )}
            </div>
          </div>
        )}
      </section>

      {/* Manual stem preview & upload */}
      <section style={{ marginBottom: 32 }}>
        <h2 style={{ fontSize: 16, marginBottom: 4 }}>Manual Upload</h2>
        <p style={{ fontSize: 12, color: "#666", margin: "0 0 10px" }}>
          Pick a folder containing <code>melody.mp3</code>,{" "}
          <code>vocals.mp3</code>, <code>bass.mp3</code>, <code>drums.mp3</code>
          .
        </p>
        <button onClick={handlePickAndLoad}>Select Stem Folder</button>
        {folder && (
          <p style={{ marginTop: 8, fontSize: 12, color: "#666" }}>
            Loaded: {folder}
          </p>
        )}
        <div style={{ marginTop: 10 }}>
          <button onClick={play} style={{ marginRight: 8 }}>
            ▶ Play
          </button>
          <button onClick={pause}>⏸ Pause</button>
        </div>
        <div
          style={{
            display: "flex",
            gap: 40,
            marginTop: 16,
            opacity: uploading ? 0.5 : 1,
            pointerEvents: uploading ? "none" : "auto",
          }}
        >
          <StemSlider stem="drums" />
          <StemSlider stem="bass" />
          <StemSlider stem="melody" />
          <StemSlider stem="vocals" />
        </div>
        <div
          style={{
            display: "flex",
            gap: 8,
            alignItems: "center",
            marginTop: 14,
            marginBottom: 10,
          }}
        >
          <label style={{ fontSize: 13 }}>Track name:</label>
          <input
            type="text"
            value={trackName}
            onChange={(e) => setTrackName(e.target.value)}
            placeholder="e.g. My Song"
            style={{ padding: "4px 8px", width: 220 }}
          />
        </div>
        <label style={{ fontSize: 13, display: "flex", alignItems: "center", gap: 6 }}>
          <input
            type="checkbox"
            checked={newAlbum}
            onChange={(e) => setNewAlbum(e.target.checked)}
          />
          Create new album
        </label>
        <button
          onClick={handleUpload}
          disabled={!folder || !selectedDevice || uploading}
        >
          {uploading ? "Uploading…" : "Upload Stems"}
        </button>
        {uploading && (
          <span style={{ marginLeft: 12, color: "orange" }}>Please wait…</span>
        )}
      </section>

      {/* Album / track browser */}
      {connected && (
        <section>
          <h2 style={{ fontSize: 16, marginBottom: 12 }}>On Device</h2>
          {loadingTracks && <p style={{ color: "#888" }}>Loading…</p>}
          {!loadingTracks && albums.length === 0 && (
            <p style={{ color: "#888", fontSize: 13 }}>
              No tracks found on device.
            </p>
          )}
          {albums.map((album) => (
            <div
              key={album.album_id}
              style={{
                border: "1px solid #333",
                borderRadius: 8,
                marginBottom: 16,
                overflow: "hidden",
              }}
            >
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  padding: "8px 14px",
                  background: "#1a1a1a",
                }}
              >
                <span style={{ fontWeight: 600, fontSize: 14, color: "#fff" }}>
                  {album.album_id}
                </span>
              </div>
              {album.tracks.map((track: TrackInfo) => (
                <div
                  key={track.track_id}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                    padding: "8px 14px",
                    borderTop: "1px solid #2a2a2a",
                  }}
                >
                  <div
                    style={{ display: "flex", alignItems: "center", gap: 10 }}
                  >
                    <div
                      style={{
                        width: 28,
                        height: 28,
                        borderRadius: 4,
                        flexShrink: 0,
                        background: `linear-gradient(135deg, ${
                          track.colours[0] ?? "#444"
                        } 50%, ${track.colours[1] ?? "#888"} 50%)`,
                      }}
                    />
                    <div>
                      <div style={{ fontWeight: 500, fontSize: 14 }}>
                        {track.title}
                      </div>
                      <div style={{ fontSize: 12, color: "#888" }}>
                        {track.artist} · {track.track_id}
                      </div>
                    </div>
                  </div>
                  <button
                    onClick={() =>
                      handleDeleteTrack(
                        track.album_id,
                        track.track_id,
                        track.title
                      )
                    }
                    style={{
                      color: "#e05555",
                      background: "none",
                      border: "none",
                      cursor: "pointer",
                      fontSize: 12,
                    }}
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
  );
}

// Sub-components

function SplitterBadge({ status }: { status: string; detail: string }) {
  const cfg: Record<string, { bg: string; color: string; border: string }> = {
    unknown:          { bg: "#1a1a1a", color: "#555", border: "#333" },
    checking:         { bg: "#1a1a1a", color: "#888", border: "#333" },
    ok:               { bg: "#1a1a1a", color: "#32a850", border: "#333" },
    "needs-download": { bg: "#1a1a1a", color: "orange", border: "#333" },
    downloading:      { bg: "#1a1a1a", color: "#ccc", border: "#333" },
    missing:          { bg: "#1a1a1a", color: "#ff2c2c", border: "#333" },
  };
  const label: Record<string, string> = {
    unknown:          "Splitter: unknown",
    checking:         "Checking…",
    ok:               "Ready",
    "needs-download": "Download required",
    downloading:      "Downloading…",
    missing:          "Download failed",
  };
  const c = cfg[status] ?? cfg.unknown;
  return (
    <span style={{
      fontSize: 11, padding: "2px 8px", borderRadius: 99,
      background: c.bg, color: c.color, border: `1px solid ${c.border}`,
    }}>
      {label[status] ?? status}
    </span>
  );
}

function Spinner() {
  return (
    <>
      <style>{`@keyframes rs-spin { to { transform: rotate(360deg); } }`}</style>
      <span
        style={{
          display: "inline-block",
          width: 11,
          height: 11,
          border: "2px solid #444",
          borderTop: "2px solid orange",
          borderRadius: "50%",
          animation: "rs-spin 0.7s linear infinite",
          marginRight: 6,
          verticalAlign: "middle",
        }}
      />
    </>
  );
}