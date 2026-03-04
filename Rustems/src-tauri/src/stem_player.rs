use rusb::{Context, UsbContext};
use std::time::Duration;
use std::sync::Mutex;
use std::fs;
use tokio::task;

// ── Command bytes (all verified against Wireshark capture) ───────────────────

const MSG_CONTROL: u8        = 0x04;
const CTRL_PING: u8          = 0x03;
const CTRL_REQUEST_ALBUM: u8 = 0x05;
const CTRL_REQUEST_TRACK: u8 = 0x06;

const MSG_FILE_HEADER: u8    = 0x06;
const MSG_FILE_BODY: u8      = 0x07;

const RECORD_ALBUM: &str     = "RECORD";
const CHUNK_SIZE: usize      = 8192;

pub struct DeviceState(pub Mutex<Option<rusb::DeviceHandle<rusb::Context>>>);

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

    let proddata = fs::read(find_device_drive()?)
        .map_err(|e| format!("Failed to read PRODDATA.DAT: {}", e))?;
    if proddata.len() < 8 { return Err("PRODDATA.DAT too small".to_string()); }
    let album_id = format!("A{}", u32::from_le_bytes([proddata[0], proddata[1], proddata[2], proddata[3]]) + 1);
    let track_id = format!("T{}", u32::from_le_bytes([proddata[4], proddata[5], proddata[6], proddata[7]]) + 1);
    eprintln!("[upload] New album: {}, track: {}", album_id, track_id);

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
    let _listing3_pkt = pass2_result.ok_or("Pass 2: RECORD album not found in listing")?;

    // Device is now primed — upload begins immediately after the final ping.

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
    let mut track_body = serde_json::to_vec(&serde_json::json!({
        "TrackColour": ["#B3BD2A", "#212961"],
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

// ── Drive discovery ───────────────────────────────────────────────────────────

fn find_device_drive() -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        for letter in b'D'..=b'Z' {
            let path = std::path::PathBuf::from(format!("{}:/PRODDATA.DAT", letter as char));
            if path.exists() { return Ok(path); }
        }
        Err("Stem Player drive not found (PRODDATA.DAT missing D–Z)".to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        for base in &["/media", "/Volumes"] {
            if let Ok(entries) = fs::read_dir(base) {
                for entry in entries.flatten() {
                    let path = entry.path().join("PRODDATA.DAT");
                    if path.exists() { return Ok(path); }
                }
            }
        }
        Err("Stem Player drive not found (PRODDATA.DAT missing under /media or /Volumes)".to_string())
    }
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
    handle.write_bulk(ep_out, &build_message(0x02, &[]), timeout)
        .map_err(|e| format!("CONNECT write failed: {}", e))?;
    read_response(&handle, ep_in, timeout)?;

    handle.write_bulk(ep_out, &build_message(MSG_CONTROL, &[0x12]), timeout)
        .map_err(|e| format!("CHALLENGE write failed: {}", e))?;
    read_response(&handle, ep_in, timeout)?;

    handle.write_bulk(ep_out, &[0x01, 0x00, 0x00], timeout)
        .map_err(|e| format!("handshake ACK failed: {}", e))?;

    // ── Initial sync (connect-time) ───────────────────────────────────────────
    // Just read the track listing so we know what's on device.
    // If device is stuck in upload mode from a prior failed session, the ping
    // will return [02 00 01 03] instead of a listing. Drain and retry.
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
    state: tauri::State<'_, DeviceState>,
) -> Result<(), String> {
    let folder_path = std::path::Path::new(&folder);
    let track_name  = folder_path.file_name()
        .and_then(|n| n.to_str()).unwrap_or("track").to_string();

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

    task::spawn_blocking(move || do_upload(handle, 0x01, 0x81, track_name, stem_data))
        .await
        .map_err(|e| format!("Upload task panicked: {}", e))?
}

#[tauri::command]
pub fn check_device_state(state: tauri::State<'_, DeviceState>) -> String {
    match state.0.try_lock() {
        Ok(g)  => if g.is_some() { "Connected".into() } else { "Not connected".into() },
        Err(_) => "DEADLOCK: mutex locked".into(),
    }
}