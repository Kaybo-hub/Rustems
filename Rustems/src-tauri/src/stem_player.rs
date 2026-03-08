use rusb::{Context, UsbContext};
use std::time::Duration;
use std::sync::Mutex;
use std::fs;
use tokio::task;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use tauri::State;

// ── Command bytes (all verified against Wireshark capture) ───────────────────

// Message types (outer frame byte, verified against emulator source)
const MSG_CONNECT: u8        = 0x02;
const MSG_DISCONNECT: u8     = 0x03; // standalone disconnect frame — no payload
const MSG_CONTROL: u8        = 0x04;
const MSG_FILE_HEADER: u8    = 0x06;
const MSG_FILE_BODY: u8      = 0x07;

// Control subtypes (payload[0] inside a MSG_CONTROL frame)
const CTRL_PING: u8          = 0x03; // GET_TRACKS_INFO
const CTRL_REQUEST_ALBUM: u8 = 0x05; // GET_ALBUM_CONFIG
const CTRL_REQUEST_TRACK: u8 = 0x06; // GET_TRACK_CONFIG
const CTRL_DELETE_ALBUM: u8  = 0x09; // DELETE_ALBUM
const CTRL_DELETE_TRACK: u8  = 0x0A; // DELETE_TRACK

const RECORD_ALBUM: &str     = "RECORD";
const CHUNK_SIZE: usize      = 8192;

pub struct DeviceState(pub Mutex<Option<rusb::DeviceHandle<rusb::Context>>>);

// ── Public data types returned to the frontend ────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct TrackInfo {
    pub album_id: String,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub colours: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AlbumInfo {
    pub album_id: String,
    pub tracks: Vec<TrackInfo>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct StorageInfo {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
}

// ── Packet framing ────────────────────────────────────────────────────────────
//
// All packets: [total_len : u16 LE] [cmd : u8] [payload...]
// total_len = 1 (cmd) + payload length — does NOT include the 2 length bytes.

fn build_message(msg_type: u8, payload: &[u8]) -> Vec<u8> {
    let total_len = (1 + payload.len()) as u16;
    let mut msg = Vec::with_capacity(3 + payload.len());
    msg.push((total_len & 0xFF) as u8);
    msg.push((total_len >> 8)   as u8);
    msg.push(msg_type);
    msg.extend_from_slice(payload);
    msg
}

fn build_upload_init(metadata: &serde_json::Value) -> Vec<u8> {
    let mut json_bytes = serde_json::to_vec(metadata).unwrap();
    json_bytes.push(0x00);
    let total_len = (1 + json_bytes.len()) as u16;
    let mut msg = Vec::with_capacity(3 + json_bytes.len());
    msg.push((total_len & 0xFF) as u8);
    msg.push((total_len >> 8)   as u8);
    msg.push(MSG_FILE_HEADER);
    msg.extend_from_slice(&json_bytes);
    msg
}

/// 8-byte chunk header, verified against capture.
/// Packet = [len_field u16 LE][0x07][data_size u32 LE][data...]
/// len_field = data_size + 6  (counts cmd byte + 4 size bytes + 1 extra)
fn build_chunk_header(data_size: usize) -> [u8; 8] {
    let len_field = (data_size + 6) as u16;
    let size = data_size as u32;
    [
        (len_field & 0xFF) as u8,
        (len_field >> 8)   as u8,
        MSG_FILE_BODY,
        (size        & 0xFF) as u8,
        ((size >> 8)  & 0xFF) as u8,
        ((size >> 16) & 0xFF) as u8,
        ((size >> 24) & 0xFF) as u8,
        0x00,
    ]
}

// ── I/O helpers ───────────────────────────────────────────────────────────────

fn read_response(
    handle: &rusb::DeviceHandle<rusb::Context>,
    ep_in: u8,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let mut data = Vec::with_capacity(4096);
    let mut buf  = vec![0u8; 4096];
    loop {
        let n = handle.read_bulk(ep_in, &mut buf, timeout)
            .map_err(|e| format!("Read failed: {}", e))?;
        if n == 0 { break; }
        data.extend_from_slice(&buf[..n]);
        if data.len() >= 2 {
            let total_len = data[0] as usize + (data[1] as usize) * 256;
            let expected  = 2 + total_len;
            if data.len() >= expected {
                data.truncate(expected);
                return Ok(data);
            }
        }
    }
    Ok(data)
}

fn send_ack(
    handle: &rusb::DeviceHandle<rusb::Context>,
    ep_out: u8,
    timeout: Duration,
) -> Result<(), String> {
    handle.write_bulk(ep_out, &[0x01, 0x00, 0x00], timeout)
        .map(|_| ())
        .map_err(|e| format!("ACK write failed: {}", e))
}

fn expect_ack(
    handle: &rusb::DeviceHandle<rusb::Context>,
    ep_in: u8,
    timeout: Duration,
    label: &str,
) -> Result<(), String> {
    let resp = read_response(handle, ep_in, timeout)?;
    if resp != [0x01, 0x00, 0x00] {
        return Err(format!("{}: expected ACK [01 00 00], got {:02x?}", label, resp));
    }
    Ok(())
}

fn drain_stale(handle: &rusb::DeviceHandle<rusb::Context>, ep_in: u8) {
    let mut buf  = vec![0u8; 4096];
    let short_to = Duration::from_millis(20);
    for _ in 0..16 {
        match handle.read_bulk(ep_in, &mut buf, short_to) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

// ── Device read protocol ──────────────────────────────────────────────────────
//
// HOST → REQUEST  (MSG_CONTROL + CTRL_REQUEST_ALBUM/TRACK + json + 0x00)
// DEV  → READY    ([02 00 05 05] album  or  [02 00 05 06] track)
// HOST → ACK      ([01 00 00])
// DEV  → HEADER   (cmd=0x06, json containing "size")
// HOST → ACK      ([01 00 00])
// DEV  → DATA     (cmd=0x07, file bytes)
// HOST → (send next request — NO trailing ACK after DATA)

fn read_device_file(
    handle: &rusb::DeviceHandle<rusb::Context>,
    ep_out: u8,
    ep_in: u8,
    request: Vec<u8>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    handle.write_bulk(ep_out, &request, timeout)
        .map_err(|e| format!("request write failed: {}", e))?;

    let ready = read_response(handle, ep_in, timeout)?;
    if ready.len() < 3 || ready[0] != 0x02 || ready[1] != 0x00 || ready[2] != 0x05 {
        return Err(format!("expected READY, got {:02x?}", ready));
    }
    send_ack(handle, ep_out, timeout)?;

    let header = read_response(handle, ep_in, timeout)?;
    if header.len() < 3 || header[2] != 0x06 {
        return Err(format!("expected FILE_HEADER, got {:02x?}", &header[..header.len().min(8)]));
    }
    send_ack(handle, ep_out, timeout)?;

    let data_pkt = read_response(handle, ep_in, timeout)?;
    if data_pkt.len() < 3 || data_pkt[2] != 0x07 {
        return Err(format!("expected FILE_DATA, got {:02x?}", &data_pkt[..data_pkt.len().min(8)]));
    }

    Ok(if data_pkt.len() > 8 { data_pkt[8..].to_vec() } else { vec![] })
}

fn parse_track_listing(pkt: &[u8]) -> Vec<(String, Vec<String>)> {
    if pkt.len() < 5 { return vec![]; }
    if pkt[2] != 0x05 || pkt[3] != 0x03 { return vec![]; }
    let total_len  = pkt[0] as usize + (pkt[1] as usize) * 256;
    let null_index = (1 + total_len).min(pkt.len());
    if null_index <= 4 { return vec![]; }
    let json_str = match std::str::from_utf8(&pkt[4..null_index]) {
        Ok(s) => s, Err(_) => return vec![],
    };
    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v, Err(_) => return vec![],
    };
    parsed["l"].as_array().unwrap_or(&vec![]).iter().filter_map(|e| {
        let album  = e["a"].as_str()?.to_string();
        let tracks = e["c"].as_array().unwrap_or(&vec![])
            .iter().filter_map(|t| t["t"].as_str().map(|s| s.to_string())).collect();
        Some((album, tracks))
    }).collect()
}

/// Parse a track-config JSON blob into a TrackInfo.
/// Falls back to placeholder strings if fields are missing.
fn parse_track_config(album_id: &str, track_id: &str, raw: &[u8]) -> TrackInfo {
    // Strip trailing null byte if present
    let json_bytes = if raw.last() == Some(&0x00) { &raw[..raw.len()-1] } else { raw };

    let (title, artist, colours) = match serde_json::from_slice::<serde_json::Value>(json_bytes) {
        Ok(v) => {
            let title  = v["metadata"]["title"].as_str().unwrap_or(track_id).to_string();
            let artist = v["metadata"]["artist"].as_str().unwrap_or("Unknown").to_string();
            let colours = v["TrackColour"].as_array()
                .map(|a| a.iter().filter_map(|c| c.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_else(|| vec!["#ffffff".to_string(), "#ffffff".to_string()]);
            (title, artist, colours)
        }
        Err(_) => (track_id.to_string(), "Unknown".to_string(), vec!["#ffffff".to_string(), "#ffffff".to_string()]),
    };

    TrackInfo { album_id: album_id.to_string(), track_id: track_id.to_string(), title, artist, colours }
}

fn do_ping(
    handle: &rusb::DeviceHandle<rusb::Context>,
    ep_out: u8,
    ep_in: u8,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    handle.write_bulk(ep_out, &build_message(MSG_CONTROL, &[CTRL_PING]), timeout)
        .map_err(|e| format!("PING write failed: {}", e))?;
    read_response(handle, ep_in, timeout)
}

// ── Sync protocol (verified from Wireshark capture) ──────────────────────────
//
// PASS 1:
//   ping → listing
//   for each album in order:
//     if RECORD: send REQ_ALBUM RECORD → get [02 00 01 03] → send PING (no ACK) → get new listing → STOP
//     else: read album-config + all track-configs
//
// PASS 2 (called with the listing from pass-1's final ping):
//   for each album in order:
//     if RECORD:
//       send REQ_ALBUM RECORD → get [02 00 01 03]
//       send REQ_TRACK RECORD/T1 → read as normal file
//       continue reading any albums that come AFTER RECORD in the listing
//       send PING → get listing → return
//     else: read album-config + all track-configs
//
// After pass 2 returns, device is primed and upload can begin immediately.

/// Run one pass of the sync loop.
/// `listing` is the track-listing packet already received (from a prior ping).
/// Returns `Ok(Some(listing))` if RECORD was hit and a fresh ping was done,
/// or `Ok(None)` if the listing was exhausted without hitting RECORD.
fn sync_pass(
    handle: &rusb::DeviceHandle<rusb::Context>,
    ep_out: u8,
    ep_in: u8,
    timeout: Duration,
    listing: &[(String, Vec<String>)],
    is_pass2: bool,
) -> Result<Option<Vec<u8>>, String> {
    let mut after_record = false;
    let mut hit_record   = false;

    for (album_id, track_ids) in listing {
        if album_id == RECORD_ALBUM {
            hit_record = true;
            eprintln!("[sync] Querying RECORD...");

            // Send RECORD album request
            handle.write_bulk(ep_out, &build_message(MSG_CONTROL, &{
                let json = format!("{{\"album\":\"{}\"}}", RECORD_ALBUM);
                let mut p = vec![CTRL_REQUEST_ALBUM];
                p.extend_from_slice(json.as_bytes()); p.push(0x00); p
            }), timeout).map_err(|e| format!("RECORD query failed: {}", e))?;

            let resp = read_response(handle, ep_in, timeout)?;
            eprintln!("[sync] RECORD response: {:02x?}", &resp[..resp.len().min(6)]);

            if is_pass2 {
                // Pass 2: read RECORD/T1 as a normal file, then continue with remaining albums
                let t1 = read_device_file(handle, ep_out, ep_in,
                    build_message(MSG_CONTROL, &{
                        let json = format!("{{\"album\":\"{}\",\"track\":\"T1\"}}", RECORD_ALBUM);
                        let mut p = vec![CTRL_REQUEST_TRACK];
                        p.extend_from_slice(json.as_bytes()); p.push(0x00); p
                    }), timeout);
                eprintln!("[sync] RECORD/T1: {:?}", t1.as_ref().map(|v| v.len()));
                after_record = true;
            } else {
                // Pass 1: immediately ping to get a fresh listing, return it
                eprintln!("[sync] Pass 1 RECORD done, re-pinging...");
                let new_listing = do_ping(handle, ep_out, ep_in, timeout)?;
                return Ok(Some(new_listing));
            }
            continue;
        }

        if !is_pass2 && !after_record {
            // Pass 1: read everything before RECORD
        } else if is_pass2 {
            // Pass 2: read everything (before and after RECORD)
        } else {
            // Pass 1 after RECORD shouldn't happen (we return early)
            break;
        }

        eprintln!("[sync] Reading album-config {}", album_id);
        read_device_file(handle, ep_out, ep_in,
            build_message(MSG_CONTROL, &{
                let json = format!("{{\"album\":\"{}\"}}", album_id);
                let mut p = vec![CTRL_REQUEST_ALBUM];
                p.extend_from_slice(json.as_bytes()); p.push(0x00); p
            }), timeout)?;

        for track_id in track_ids {
            read_device_file(handle, ep_out, ep_in,
                build_message(MSG_CONTROL, &{
                    let json = format!("{{\"album\":\"{}\",\"track\":\"{}\"}}", album_id, track_id);
                    let mut p = vec![CTRL_REQUEST_TRACK];
                    p.extend_from_slice(json.as_bytes()); p.push(0x00); p
                }), timeout)?;
        }
    }

    if is_pass2 && hit_record {
        // After reading all albums including those after RECORD, do final ping
        eprintln!("[sync] Pass 2 complete, final ping...");
        let listing_pkt = do_ping(handle, ep_out, ep_in, timeout)?;
        return Ok(Some(listing_pkt));
    }

    Ok(None)
}

// ── File upload ───────────────────────────────────────────────────────────────
//
// Upload protocol (verified from capture):
//   HOST → UPLOAD_INIT (MSG_FILE_HEADER + json + 0x00)
//   DEV  → ACK [01 00 00]
//   HOST → CHUNK (8-byte header + up to 8192 bytes data)
//   DEV  → ACK [01 00 00]
//   ... repeat chunks until all data sent ...
//   (No status packets [02 00 01 xx] appear in a working upload)

fn send_file(
    handle: &rusb::DeviceHandle<rusb::Context>,
    ep_out: u8,
    ep_in: u8,
    mut metadata: serde_json::Value,
    body: &[u8],
    timeout: Duration,
    label: &str,
) -> Result<(), String> {
    metadata["size"] = serde_json::Value::from(body.len());

    handle.write_bulk(ep_out, &build_upload_init(&metadata), timeout)
        .map_err(|e| format!("{} init write failed: {}", label, e))?;
    expect_ack(handle, ep_in, timeout, &format!("{} init", label))?;

    let mut offset = 0;
    while offset < body.len() {
        let end   = (offset + CHUNK_SIZE).min(body.len());
        let chunk = &body[offset..end];
        let hdr   = build_chunk_header(chunk.len());
        let mut pkt = Vec::with_capacity(8 + chunk.len());
        pkt.extend_from_slice(&hdr);
        pkt.extend_from_slice(chunk);

        handle.write_bulk(ep_out, &pkt, timeout)
            .map_err(|e| format!("{} chunk @{} write failed: {}", label, offset, e))?;
        expect_ack(handle, ep_in, timeout, &format!("{} chunk @{}", label, offset))?;
        offset = end;
    }
    Ok(())
}

// ── Upload orchestration ──────────────────────────────────────────────────────

fn do_upload(
    handle: rusb::DeviceHandle<rusb::Context>,
    ep_out: u8,
    ep_in: u8,
    track_name: String,
    stem_data: Vec<(usize, Vec<u8>)>,
) -> Result<(), String> {
    let sync_timeout  = Duration::from_secs(10);
    let audio_timeout = Duration::from_secs(120);

    // ── Pass 1: sync until RECORD, then re-ping ───────────────────────────────
    eprintln!("[upload] Pass 1: initial sync...");
    let listing_pkt = do_ping(&handle, ep_out, ep_in, sync_timeout)?;
    let listing1 = parse_track_listing(&listing_pkt);
    eprintln!("[upload] {} album(s) on device", listing1.len());

    let pass1_result = sync_pass(&handle, ep_out, ep_in, sync_timeout, &listing1, false)?;
    let listing2_pkt = pass1_result.ok_or("Pass 1: RECORD album not found in listing")?;
    let listing2 = parse_track_listing(&listing2_pkt);

    // ── Pass 2: full sync including RECORD/T1, then final ping ────────────────
    eprintln!("[upload] Pass 2: full sync...");
    let pass2_result = sync_pass(&handle, ep_out, ep_in, sync_timeout, &listing2, true)?;
    let listing3_pkt = pass2_result.ok_or("Pass 2: RECORD album not found in listing")?;

    let listing3 = parse_track_listing(&listing3_pkt);

    let mut max_album_num = 0u32;
    let mut max_track_num = 0u32;

    for (album, tracks) in &listing3 {
        if let Some(num) = album.strip_prefix('A')
            .and_then(|s| s.parse::<u32>().ok()) {
            max_album_num = max_album_num.max(num);
        }

        for track in tracks {
            if let Some(num) = track.strip_prefix('T')
                .and_then(|s| s.parse::<u32>().ok()) {
                max_track_num = max_track_num.max(num);
            }
        }
    }

    if max_album_num == 0 {
        return Err("No valid albums found on device".into());
    }

    // Append to the highest existing album and increment track globally.
    let album_id = format!("A{}", max_album_num);
    let track_id = format!("T{}", max_track_num + 1);

    eprintln!("[upload] Allocated album: {}, track: {}", album_id, track_id);

    // Device is now primed — upload begins immediately after the final ping.

    // ── Storage check ─────────────────────────────────────────────────────────
    let storage = do_get_storage(&handle, ep_out, ep_in)?;
    // Estimate required space: sum of all stem sizes + ~1 KB for track-config.
    // Multiply by 1.1 as a safety margin for filesystem overhead.
    let required_bytes = stem_data.iter().map(|(_, d)| d.len() as u64).sum::<u64>() + 1024;
    let required_with_margin = (required_bytes as f64 * 1.1) as u64;
    if required_with_margin > storage.free_bytes {
        return Err(format!(
            "Not enough storage: need ~{} MB, only {} MB free",
            required_with_margin / 1_048_576,
            storage.free_bytes / 1_048_576
        ));
    }

    // ── Upload stems ──────────────────────────────────────────────────────────
    for (stem_num, data) in &stem_data {
        eprintln!("[upload] Uploading stem {} ({} bytes)...", stem_num, data.len());
        send_file(&handle, ep_out, ep_in,
            serde_json::json!({
                "size": data.len(),
                "type": "stem-audio-mp3",
                "track": track_id,
                "album": album_id,
                "stem": stem_num
            }),
            data, audio_timeout, &format!("stem-{}", stem_num))?;
        eprintln!("[upload] stem {} done", stem_num);
    }

    // ── Upload track-config ───────────────────────────────────────────────────
    eprintln!("[upload] Uploading track-config...");

    let (colour1, colour2) = random_colour_pair();

    let mut track_body = serde_json::to_vec(&serde_json::json!({
        "TrackColour": [colour1, colour2],
        "tempos": [{ "tempo_bpm": 120, "time_ms": 0 }],
        "TrackGain_dB": 0,
        "metadata": {
            "title": track_name,
            "artist": "Unknown",
            "global_id": uuid::Uuid::new_v4().to_string(),
            "meta_version": "1",
            "stems_version": "1"
        }
    })).unwrap();
    track_body.push(0x00);
    send_file(&handle, ep_out, ep_in,
        serde_json::json!({ "size": track_body.len(), "type": "track-config", "track": track_id, "album": album_id }),
        &track_body, sync_timeout, "track-config")?;

    eprintln!("[upload] Upload complete.");
    Ok(())
}

// ── Storage query ────────────────────────────────────────────────────────────
//
// CTRL/0x02 → {"size":"<total>","free":"<free>"}  (both values are byte counts as strings)

fn do_get_storage(
    handle: &rusb::DeviceHandle<rusb::Context>,
    ep_out: u8,
    ep_in: u8,
) -> Result<StorageInfo, String> {
    let timeout = Duration::from_secs(10);
    handle.write_bulk(ep_out, &build_message(MSG_CONTROL, &[0x02]), timeout)
        .map_err(|e| format!("storage query failed: {}", e))?;
    let resp = read_response(handle, ep_in, timeout)?;

    // Response: [02 00 05 02] + json payload
    if resp.len() < 5 {
        return Err(format!("storage response too short: {:02x?}", resp));
    }
    let total_len = resp[0] as usize + (resp[1] as usize) * 256;
    let json_end  = (2 + total_len).min(resp.len());
    let json_str  = std::str::from_utf8(&resp[4..json_end])
        .map_err(|_| "storage response not UTF-8".to_string())?
        .trim_end_matches('\0');

    let v: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("storage JSON parse failed: {}", e))?;

    let total = v["size"].as_str().unwrap_or("0").parse::<u64>().unwrap_or(0);
    let free  = v["free"].as_str().unwrap_or("0").parse::<u64>().unwrap_or(0);
    let used  = total.saturating_sub(free);

    Ok(StorageInfo { total_bytes: total, free_bytes: free, used_bytes: used })
}

// ── Fetch all albums and track metadata from the device ──────────────────────
//
// This does a lightweight read: for each real album (skipping RECORD), it
// reads the track-config of every track to extract the human-readable title.
// It does NOT read stem audio data.

fn do_list_tracks(
    handle: &rusb::DeviceHandle<rusb::Context>,
    ep_out: u8,
    ep_in: u8,
) -> Result<Vec<AlbumInfo>, String> {
    let timeout = Duration::from_secs(10);

    let listing_pkt = do_ping(handle, ep_out, ep_in, timeout)?;
    let listing = parse_track_listing(&listing_pkt);

    let mut albums: Vec<AlbumInfo> = Vec::new();

    for (album_id, track_ids) in &listing {
        if album_id == RECORD_ALBUM { continue; }

        // Read album-config (required by protocol, contents discarded)
        read_device_file(handle, ep_out, ep_in,
            build_message(MSG_CONTROL, &{
                let json = format!("{{\"album\":\"{}\"}}", album_id);
                let mut p = vec![CTRL_REQUEST_ALBUM];
                p.extend_from_slice(json.as_bytes()); p.push(0x00); p
            }), timeout)?;

        let mut tracks: Vec<TrackInfo> = Vec::new();

        for track_id in track_ids {
            let raw = read_device_file(handle, ep_out, ep_in,
                build_message(MSG_CONTROL, &{
                    let json = format!("{{\"album\":\"{}\",\"track\":\"{}\"}}", album_id, track_id);
                    let mut p = vec![CTRL_REQUEST_TRACK];
                    p.extend_from_slice(json.as_bytes()); p.push(0x00); p
                }), timeout)?;

            tracks.push(parse_track_config(album_id, track_id, &raw));
        }

        albums.push(AlbumInfo { album_id: album_id.clone(), tracks });
    }

    Ok(albums)
}

// ── Delete protocol ───────────────────────────────────────────────────────────
//
// DELETE_ALBUM = control subtype 0x09  {"album":"A1"}
// DELETE_TRACK = control subtype 0x0A  {"album":"A1","track":"T3"}
// Both follow the standard MSG_CONTROL framing and return an ACK.

fn do_delete(
    handle: &rusb::DeviceHandle<rusb::Context>,
    ep_out: u8,
    ep_in: u8,
    ctrl_byte: u8,
    json: &str,
) -> Result<(), String> {
    let timeout = Duration::from_secs(10);
    let mut payload = vec![ctrl_byte];
    payload.extend_from_slice(json.as_bytes());
    payload.push(0x00);

    handle.write_bulk(ep_out, &build_message(MSG_CONTROL, &payload), timeout)
        .map_err(|e| format!("DELETE write failed: {}", e))?;

    // Device responds with MSG_CONTROL + ctrl_byte, not a plain ACK
    let resp = read_response(handle, ep_in, timeout)?;
    if resp.len() >= 3 && resp[2] == MSG_CONTROL && resp.get(3) == Some(&ctrl_byte) {
        Ok(())
    } else {
        Err(format!("Unexpected delete response: {:02x?}", &resp[..resp.len().min(8)]))
    }
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_ = h / 60.0;
    let x = c * (1.0 - (h_ % 2.0 - 1.0).abs());

    let (r1, g1, b1) = match h_ as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    let m = l - c / 2.0;
    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

fn random_colour_pair() -> (String, String) {
    let mut rng = rand::rng();
    let h1 = rng.random_range(0.0..360.0_f32);
    let h2 = (h1 + 150.0) % 360.0;

    let s = rng.random_range(0.6..1.0_f32);
    let l = rng.random_range(0.45..0.65_f32);

    let c1 = { let (r,g,b) = hsl_to_rgb(h1, s, l); format!("#{:02X}{:02X}{:02X}", r, g, b) };
    let c2 = { let (r,g,b) = hsl_to_rgb(h2, s, l); format!("#{:02X}{:02X}{:02X}", r, g, b) };
    (c1, c2)
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn list_usb_devices() -> Result<Vec<String>, String> {
    let context = Context::new().map_err(|e| e.to_string())?;
    let mut list = Vec::new();
    for device in context.devices().map_err(|e| e.to_string())?.iter() {
        let desc = device.device_descriptor().map_err(|e| e.to_string())?;
        if desc.vendor_id() == 0x1209 && desc.product_id() == 0x572A {
            let handle = device.open().map_err(|e| e.to_string())?;
            list.push(handle.read_serial_number_string_ascii(&desc).unwrap_or_else(|_| "Unknown".into()));
        }
    }
    Ok(list)
}

#[tauri::command]
pub fn connect_usb_device(
    serial: String,
    state: tauri::State<'_, DeviceState>,
) -> Result<String, String> {
    let context = Context::new().map_err(|e| e.to_string())?;
    let device = context.devices().map_err(|e| e.to_string())?
        .iter()
        .find(|d| {
            let desc = match d.device_descriptor() { Ok(d) => d, Err(_) => return false };
            if desc.vendor_id() == 0x1209 && desc.product_id() == 0x572A {
                if let Ok(h) = d.open() {
                    return h.read_serial_number_string_ascii(&desc).unwrap_or_default() == serial;
                }
            }
            false
        })
        .ok_or("Device not found")?;

    let handle = device.open().map_err(|e: rusb::Error| e.to_string())?;

    #[cfg(not(target_os = "windows"))]
    let _ = handle.detach_kernel_driver(0);

    handle.claim_interface(0).map_err(|e| format!("claim_interface failed: {}", e))?;

    let timeout = Duration::from_secs(10);
    let ep_out: u8 = 0x01;
    let ep_in:  u8 = 0x81;

    drain_stale(&handle, ep_in);

    // ── Handshake ─────────────────────────────────────────────────────────────
    handle.write_bulk(ep_out, &build_message(MSG_CONNECT, &[]), timeout)
        .map_err(|e| format!("CONNECT write failed: {}", e))?;
    read_response(&handle, ep_in, timeout)?;

    handle.write_bulk(ep_out, &build_message(MSG_CONTROL, &[0x12]), timeout)
        .map_err(|e| format!("CHALLENGE write failed: {}", e))?;
    read_response(&handle, ep_in, timeout)?;

    handle.write_bulk(ep_out, &[0x01, 0x00, 0x00], timeout)
        .map_err(|e| format!("handshake ACK failed: {}", e))?;

    // ── Initial sync (connect-time) ───────────────────────────────────────────
    eprintln!("[connect] Initial ping...");
    let mut listing_pkt = None;
    for attempt in 0..5 {
        handle.write_bulk(ep_out, &build_message(MSG_CONTROL, &[CTRL_PING]), timeout)
            .map_err(|e| format!("ping failed: {}", e))?;
        let resp = read_response(&handle, ep_in, timeout)?;
        if resp.len() >= 4 && resp[2] == 0x05 && resp[3] == 0x03 {
            eprintln!("[connect] Got listing on attempt {}", attempt + 1);
            listing_pkt = Some(resp);
            break;
        }
        eprintln!("[connect] Unexpected ping response {:02x?}, draining and retrying...", &resp[..resp.len().min(4)]);
        drain_stale(&handle, ep_in);
        std::thread::sleep(Duration::from_millis(200));
    }

    let listing_pkt = listing_pkt.ok_or("Device did not return track listing after 5 attempts")?;
    let listing = parse_track_listing(&listing_pkt);
    eprintln!("[connect] {} album(s) on device", listing.len());

    *state.0.lock().unwrap() = Some(handle);
    Ok("Connected".to_string())
}

#[tauri::command]
pub async fn upload_stems(
    folder: String,
    track_name: String,
    state: tauri::State<'_, DeviceState>,
) -> Result<(), String> {
    let folder_path = std::path::Path::new(&folder);

    // Use provided track_name; fall back to folder name if blank
    let resolved_name = if track_name.trim().is_empty() {
        folder_path.file_name()
            .and_then(|n| n.to_str()).unwrap_or("track").to_string()
    } else {
        track_name.trim().to_string()
    };

    let mut stem_data: Vec<(usize, Vec<u8>)> = Vec::new();
    for (i, name) in ["melody", "vocals", "bass", "drums"].iter().enumerate() {
        let path = folder_path.join(format!("{}.mp3", name));
        let data = fs::read(&path).map_err(|e| format!("Failed to read {}.mp3: {}", name, e))?;
        eprintln!("[upload] Read {} — {} bytes", name, data.len());
        stem_data.push((i + 1, data));
    }

    let handle = state.0.lock()
        .map_err(|_| "Mutex poisoned".to_string())?
        .take()
        .ok_or("Device not connected")?;

    task::spawn_blocking(move || do_upload(handle, 0x01, 0x81, resolved_name, stem_data))
        .await
        .map_err(|e| format!("Upload task panicked: {}", e))?
}

/// Returns all albums and their tracks with human-readable titles read from
/// each track's config. Requires the device to be connected.
#[tauri::command]
pub fn list_device_tracks(
    state: tauri::State<'_, DeviceState>,
) -> Result<Vec<AlbumInfo>, String> {
    let guard = state.0.lock().map_err(|_| "Mutex poisoned".to_string())?;
    let handle = guard.as_ref().ok_or("Device not connected")?;
    do_list_tracks(handle, 0x01, 0x81)
}

/// Delete a single track from the device.
#[tauri::command]
pub fn delete_track(
    album_id: String,
    track_id: String,
    state: tauri::State<'_, DeviceState>,
) -> Result<(), String> {
    let guard = state.0.lock().map_err(|_| "Mutex poisoned".to_string())?;
    let handle = guard.as_ref().ok_or("Device not connected")?;
    let json = format!("{{\"album\":\"{}\",\"track\":\"{}\"}}", album_id, track_id);
    do_delete(handle, 0x01, 0x81, CTRL_DELETE_TRACK, &json)
}

/// Delete an entire album and all its tracks from the device.
#[tauri::command]
pub fn delete_album(
    album_id: String,
    state: tauri::State<'_, DeviceState>,
) -> Result<(), String> {
    let guard = state.0.lock().map_err(|_| "Mutex poisoned".to_string())?;
    let handle = guard.as_ref().ok_or("Device not connected")?;
    let json = format!("{{\"album\":\"{}\"}}", album_id);
    do_delete(handle, 0x01, 0x81, CTRL_DELETE_ALBUM, &json)
}

/// Query device storage (total, used, free bytes).
#[tauri::command]
pub fn get_storage_info(
    state: tauri::State<'_, DeviceState>,
) -> Result<StorageInfo, String> {
    let guard = state.0.lock().map_err(|_| "Mutex poisoned".to_string())?;
    let handle = guard.as_ref().ok_or("Device not connected")?;
    do_get_storage(handle, 0x01, 0x81)
}

#[tauri::command]
pub fn disconnect_device(state: State<DeviceState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    if let Some(handle) = guard.take() {
        do_disconnect(handle);
    }
    Ok(())
}

pub fn do_disconnect(handle: rusb::DeviceHandle<rusb::Context>) {
    let timeout = Duration::from_millis(500);
    let msg = build_message(MSG_DISCONNECT, &[]);
    let _ = handle.write_bulk(0x01, &msg, timeout);
    drain_stale(&handle, 0x81);
    let _ = handle.release_interface(0);
    drop(handle);
}