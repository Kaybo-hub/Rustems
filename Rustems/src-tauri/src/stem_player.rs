use rusb::{Context, UsbContext};
use std::time::Duration;
use std::sync::Mutex;

const MSG_ACK: u8 = 0;
const MSG_CONNECT: u8 = 2;
const MSG_CONTROL: u8 = 4;
const CTRL_CHALLENGE: u8 = 18;

pub struct DeviceState(pub Mutex<Option<rusb::DeviceHandle<rusb::Context>>>);

fn build_message(msg_type: u8, payload: &[u8]) -> Vec<u8> {
    let l = payload.len() + 1;
    let mut msg = vec![
        (l & 0xFF) as u8,
        ((l >> 8) & 0xFF) as u8,
        msg_type,
    ];
    msg.extend_from_slice(payload);
    msg
}

fn read_response(handle: &rusb::DeviceHandle<rusb::Context>, endpoint: u8, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; 64];
    match handle.read_bulk(endpoint, &mut buf, timeout) {
        Ok(n) => Ok(buf[..n].to_vec()),
        Err(e) => Err(format!("Read failed: {}", e)),
    }
}

#[tauri::command]
pub fn list_usb_devices() -> Result<Vec<String>, String> {
    let context = Context::new().map_err(|e| e.to_string())?;
    let devices = context.devices().map_err(|e| e.to_string())?;
    let mut list = Vec::new();

    for device in devices.iter() {
        let desc = device.device_descriptor().map_err(|e| e.to_string())?;
        if desc.vendor_id() == 0x1209 && desc.product_id() == 0x572A {
            let handle = device.open().map_err(|e| e.to_string())?;
            let sn = handle.read_serial_number_string_ascii(&desc)
                .unwrap_or_else(|_| "Unknown".into());
            list.push(sn);
        }
    }
    Ok(list)
}

#[tauri::command]
pub fn connect_usb_device(
    serial: String,
    state: tauri::State<'_, DeviceState>
) -> Result<String, String> {
    let context = Context::new().map_err(|e| e.to_string())?;

    let device = context.devices().map_err(|e| e.to_string())?
        .iter()
        .find(|d| {
            let desc = match d.device_descriptor() {
                Ok(d) => d,
                Err(_) => return false,
            };
            if desc.vendor_id() == 0x1209 && desc.product_id() == 0x572A {
                if let Ok(handle) = d.open() {
                    let sn = handle.read_serial_number_string_ascii(&desc).unwrap_or_default();
                    return sn == serial;
                }
            }
            false
        })
        .ok_or("Device not found")?;

    let handle = device.open().map_err(|e: rusb::Error| e.to_string())?;

    #[cfg(not(target_os = "windows"))]
    let _ = handle.detach_kernel_driver(0);

    handle.claim_interface(0).map_err(|e| format!("claim_interface failed: {}", e))?;

    let timeout = Duration::from_millis(500);
    let ep_out: u8 = 0x01;
    let ep_in: u8  = 0x81;

    let connect_msg = build_message(MSG_CONNECT, &[]);
    handle.write_bulk(ep_out, &connect_msg, timeout)
        .map_err(|e| format!("write CONNECT failed: {}", e))?;

    read_response(&handle, ep_in, timeout)?;

    let challenge_msg = build_message(MSG_CONTROL, &[CTRL_CHALLENGE]);
    handle.write_bulk(ep_out, &challenge_msg, timeout)
        .map_err(|e| format!("write CHALLENGE failed: {}", e))?;

    read_response(&handle, ep_in, timeout)?;

    let ack_msg = build_message(MSG_ACK, &[]);
    handle.write_bulk(ep_out, &ack_msg, timeout)
        .map_err(|e| format!("write final ACK failed: {}", e))?;

    *state.0.lock().unwrap() = Some(handle);

    Ok("Connected".to_string())
}

#[tauri::command]
pub fn set_led_color(
    state: tauri::State<'_, DeviceState>,
    _serial: String,
    red: u8,
    green: u8,
    blue: u8
) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    let handle = guard.as_mut().ok_or("Device not connected. Click Connect first.")?;

    let payload = vec![0x05, 0x00, 0x04, 0x01, red, green, blue];
    handle.write_bulk(
        0x01,
        &payload,
        Duration::from_millis(100)
    ).map_err(|e| e.to_string())?;

    Ok(())
}