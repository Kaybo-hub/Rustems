use serde::{Deserialize, Serialize};
use mp3lame_encoder::{Builder, FlushNoGap, InterleavedPcm};
use stem_splitter_core::{split_file, SplitOptions};
use tokio::task;

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

// Rubato for resampling
use rubato::{FftFixedIn, Resampler};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SplitResult {
    pub output_dir: String,
    pub track_name: String,
    pub stems: Vec<String>, // in order [vocals, drums, bass, melody].
}

// ── Audio normalisation ───────────────────────────────────────────────────────
//
// Decodes any Symphonia-supported format to interleaved f32 samples at the
// source sample rate, then resamples to 44100 Hz if necessary, then writes a
// clean 44100 Hz stereo 16-bit PCM WAV to `dest_path`.

const TARGET_SR: u32 = 44100;

fn normalise_to_wav(src_path: &str, dest_path: &str) -> Result<(), String> {
    // Decode
    let file = std::fs::File::open(src_path)
        .map_err(|e| format!("Cannot open '{}': {}", src_path, e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(src_path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Probe failed: {}", e))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or("No audio track found")?
        .clone();

    let src_sr = track.codec_params.sample_rate.unwrap_or(TARGET_SR);
    let src_channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Decoder init failed: {}", e))?;

    // Collect all decoded samples as interleaved f32
    let mut interleaved: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(format!("Packet read error: {}", e)),
        };

        if packet.track_id() != track_id { continue; }

        match decoder.decode(&packet) {
            Ok(buf) => convert_to_f32(&buf, src_channels, &mut interleaved),
            Err(symphonia::core::errors::Error::IoError(_)) => break,
            Err(symphonia::core::errors::Error::DecodeError(e)) => {
                eprintln!("[normalise] decode warning: {}", e);
                continue;
            }
            Err(e) => return Err(format!("Decode error: {}", e)),
        }
    }

    if interleaved.is_empty() {
        return Err("No audio samples decoded".into());
    }

    eprintln!(
        "[normalise] Decoded {} frames @ {}Hz, {} ch",
        interleaved.len() / src_channels.max(1),
        src_sr,
        src_channels
    );

    // Convert to planar stereo
    let frame_count = interleaved.len() / src_channels.max(1);
    let (mut left, mut right): (Vec<f32>, Vec<f32>) = match src_channels {
        1 => {
            // Mono -> duplicate to stereo
            let l = interleaved.clone();
            let r = interleaved;
            (l, r)
        }
        2 => {
            let mut l = Vec::with_capacity(frame_count);
            let mut r = Vec::with_capacity(frame_count);
            for chunk in interleaved.chunks_exact(2) {
                l.push(chunk[0]);
                r.push(chunk[1]);
            }
            (l, r)
        }
        n => {
            // > 2 channels: take first two
            let mut l = Vec::with_capacity(frame_count);
            let mut r = Vec::with_capacity(frame_count);
            for chunk in interleaved.chunks_exact(n) {
                l.push(chunk[0]);
                r.push(chunk[1]);
            }
            (l, r)
        }
    };

    // Resample to 44100 Hz if needed
    if src_sr != TARGET_SR {
        eprintln!("[normalise] Resampling {}Hz -> {}Hz", src_sr, TARGET_SR);
        let chunk_size = 1024usize;
        let mut resampler = FftFixedIn::<f32>::new(
            src_sr as usize,
            TARGET_SR as usize,
            chunk_size,
            2,
            2,
        ).map_err(|e| format!("Resampler init failed: {}", e))?;

        let mut out_l: Vec<f32> = Vec::new();
        let mut out_r: Vec<f32> = Vec::new();

        let mut pos = 0usize;
        while pos + chunk_size <= left.len() {
            let waves = vec![
                left[pos..pos + chunk_size].to_vec(),
                right[pos..pos + chunk_size].to_vec(),
            ];
            let out = resampler.process(&waves, None)
                .map_err(|e| format!("Resample error: {}", e))?;
            out_l.extend_from_slice(&out[0]);
            out_r.extend_from_slice(&out[1]);
            pos += chunk_size;
        }

        // Flush remainder
        if pos < left.len() {
            let rem = left.len() - pos;
            let mut lpad = left[pos..].to_vec();
            let mut rpad = right[pos..].to_vec();
            lpad.resize(chunk_size, 0.0);
            rpad.resize(chunk_size, 0.0);
            let waves = vec![lpad, rpad];
            let out = resampler.process(&waves, None)
                .map_err(|e| format!("Resample flush error: {}", e))?;
            // Only take the real samples, not the zero-padding output
            let keep = (rem as f64 * TARGET_SR as f64 / src_sr as f64).ceil() as usize;
            out_l.extend_from_slice(&out[0][..keep.min(out[0].len())]);
            out_r.extend_from_slice(&out[1][..keep.min(out[1].len())]);
        }

        left  = out_l;
        right = out_r;
    }

    // Write WAV
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: TARGET_SR,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(dest_path, spec)
        .map_err(|e| format!("WAV writer init failed: {}", e))?;

    let len = left.len().min(right.len());
    for i in 0..len {
        let l = (left[i].clamp(-1.0, 1.0) * 32767.0) as i16;
        let r = (right[i].clamp(-1.0, 1.0) * 32767.0) as i16;
        writer.write_sample(l).map_err(|e| format!("WAV write error: {}", e))?;
        writer.write_sample(r).map_err(|e| format!("WAV write error: {}", e))?;
    }
    writer.finalize().map_err(|e| format!("WAV finalize error: {}", e))?;

    eprintln!("[normalise] Wrote {} frames to '{}'", len, dest_path);
    Ok(())
}

/// Convert a Symphonia AudioBufferRef to interleaved f32 samples.
fn convert_to_f32(buf: &AudioBufferRef, channels: usize, out: &mut Vec<f32>) {
    match buf {
        AudioBufferRef::F32(b) => {
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..channels.min(b.spec().channels.count()) {
                    out.push(b.chan(c)[f]);
                }
            }
        }
        AudioBufferRef::S16(b) => {
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..channels.min(b.spec().channels.count()) {
                    out.push(b.chan(c)[f] as f32 / 32768.0);
                }
            }
        }
        AudioBufferRef::S32(b) => {
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..channels.min(b.spec().channels.count()) {
                    out.push(b.chan(c)[f] as f32 / 2147483648.0);
                }
            }
        }
        AudioBufferRef::U8(b) => {
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..channels.min(b.spec().channels.count()) {
                    out.push((b.chan(c)[f] as f32 - 128.0) / 128.0);
                }
            }
        }
        AudioBufferRef::S24(b) => {
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..channels.min(b.spec().channels.count()) {
                    out.push(b.chan(c)[f].0 as f32 / 8388608.0);
                }
            }
        }
        AudioBufferRef::F64(b) => {
            let frames = b.frames();
            for f in 0..frames {
                for c in 0..channels.min(b.spec().channels.count()) {
                    out.push(b.chan(c)[f] as f32);
                }
            }
        }
        _ => {} // other formats: skip
    }
}

// ── WAV → MP3 encoder ────────────────────────────────────────────────────────
//
// Reads a 44100Hz stereo 16-bit PCM WAV (as written by normalise_to_wav / the
// splitter) and encodes it to MP3 at 320 kbps using libmp3lame.
// The device expects MP3; uploading raw WAV causes timeouts due to file size.

fn wav_to_mp3(wav_path: &str, mp3_path: &str) -> Result<(), String> {
    // Read WAV
    let mut reader = hound::WavReader::open(wav_path)
        .map_err(|e| format!("WAV open failed: {}", e))?;
    let spec = reader.spec();

    if spec.channels != 2 || spec.sample_rate != TARGET_SR {
        return Err(format!(
            "Unexpected WAV spec: {} ch @ {}Hz (expected 2ch @ 44100Hz)",
            spec.channels, spec.sample_rate
        ));
    }

    // Read all samples as i16
    let samples: Vec<i16> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("WAV read error: {}", e))?,
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.map(|f| (f.clamp(-1.0, 1.0) * 32767.0) as i16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("WAV read error: {}", e))?,
    };

    let _frame_count = samples.len() / 2;

    // Encode with mp3lame-encoder at 320kbps (statically linked, no external lib needed)
    let mut builder = Builder::new().ok_or("Failed to init LAME encoder")?;
    builder.set_num_channels(2).map_err(|e| format!("LAME channels: {:?}", e))?;
    builder.set_sample_rate(TARGET_SR).map_err(|e| format!("LAME sample_rate: {:?}", e))?;
    builder.set_brate(mp3lame_encoder::Birtate::Kbps320).map_err(|e| format!("LAME brate: {:?}", e))?;
    builder.set_quality(mp3lame_encoder::Quality::Best).map_err(|e| format!("LAME quality: {:?}", e))?;
    let mut encoder = builder.build().map_err(|e| format!("LAME build: {:?}", e))?;

    // LAME requires output buffer pre-sized to at least (samples/frame * 1.25 + 7200)
    // Use frame_count * 5 / 4 + 7200 as the recommended worst-case size per chunk.
    let sample_count = samples.len();
    let buf_size = sample_count * 5 / 8 + 7200; // /2 for stereo frames, *1.25 + 7200

    let input = InterleavedPcm(&samples);
    let mut mp3_out: Vec<std::mem::MaybeUninit<u8>> = Vec::with_capacity(buf_size);
    // mp3lame-encoder's encode() expects the Vec capacity to be available.
    // Resize to capacity so the slice passed internally has the right length.
    unsafe { mp3_out.set_len(buf_size); }
    let encoded = encoder.encode(input, &mut mp3_out)
        .map_err(|e| format!("LAME encode error: {:?}", e))?;

    let flush_size = 7200usize;
    let mut flush_out: Vec<std::mem::MaybeUninit<u8>> = Vec::with_capacity(flush_size);
    unsafe { flush_out.set_len(flush_size); }
    let flushed = encoder.flush::<FlushNoGap>(&mut flush_out)
        .map_err(|e| format!("LAME flush error: {:?}", e))?;

    // Convert MaybeUninit<u8> to u8 — safe after encode/flush guarantee initialisation
    let mut mp3_bytes: Vec<u8> = mp3_out[..encoded].iter()
        .map(|b| unsafe { b.assume_init() })
        .collect();
    let flush_bytes: Vec<u8> = flush_out[..flushed].iter()
        .map(|b| unsafe { b.assume_init() })
        .collect();
    mp3_bytes.extend_from_slice(&flush_bytes);

    std::fs::write(mp3_path, &mp3_bytes)
        .map_err(|e| format!("MP3 write failed: {}", e))?;

    eprintln!("[wav_to_mp3] {} -> {} ({} bytes)", wav_path, mp3_path, mp3_bytes.len());
    Ok(())
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn check_splitter() -> Result<String, String> {
    task::spawn_blocking(|| {
        match stem_splitter_core::ensure_model("htdemucs_ort_v1", None) {
            Ok(handle) => Ok(format!("htdemucs ready ({})", handle.local_path.display())),
            Err(e) => Err(format!("Model unavailable: {}", e)),
        }
    })
    .await
    .map_err(|e| format!("Task panicked: {}", e))?
}

#[tauri::command]
pub async fn split_stems(input_path: String) -> Result<SplitResult, String> {
    let input_path_clone = input_path.clone();

    let tmp_dir = std::env::temp_dir().join("rustems_splits");
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| format!("Failed to create temp dir: {}", e))?;

    // Normalise input to clean 44100Hz stereo WAV
    let norm_wav = tmp_dir.join("_input_normalised.wav");
    let norm_wav_str = norm_wav.to_string_lossy().to_string();
    let norm_wav_str_clone = norm_wav_str.clone();

    eprintln!("[splitter] Normalising '{}'...", input_path);
    task::spawn_blocking(move || {
        normalise_to_wav(&input_path_clone, &norm_wav_str_clone)
    })
    .await
    .map_err(|e| format!("Normalise task panicked: {}", e))??;
    eprintln!("[splitter] Normalisation complete");

    // Split the normalised WAV
    let output_dir = tmp_dir.to_string_lossy().to_string();
    let output_dir_clone = output_dir.clone();
    let norm_wav_for_split = norm_wav_str.clone();

    let result = task::spawn_blocking(move || {
        let options = SplitOptions {
            output_dir: output_dir_clone,
            model_name: "htdemucs_ort_v1".to_string(),
            manifest_url_override: None,
        };
        eprintln!("[splitter] Starting split...");
        let r = split_file(&norm_wav_for_split, options)
            .map_err(|e| format!("Split failed: {}", e))?;
        eprintln!("[splitter] Split complete");
        Ok::<stem_splitter_core::SplitResult, String>(r)
    })
    .await
    .map_err(|e| format!("Splitter task panicked: {}", e))??;

    // Clean up normalised input
    let _ = std::fs::remove_file(&norm_wav);

    // Derive track name from original input filename 
    let track_name = std::path::Path::new(&input_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("track")
        .to_string();

    // Rename outputs into canonical structure
    let stem_dir = tmp_dir.join(&track_name);
    std::fs::create_dir_all(&stem_dir)
        .map_err(|e| format!("Failed to create stem dir: {}", e))?;

    let raw = [
        ("vocals", &result.vocals_path),
        ("drums",  &result.drums_path),
        ("bass",   &result.bass_path),
        ("melody", &result.other_path),
    ];

    let mut stem_files: Vec<String> = Vec::new();
    for (name, src) in &raw {
        let src_path = std::path::Path::new(src.as_str());
        if !src_path.exists() {
            return Err(format!("stem-splitter-core did not produce '{}' stem (looked for: {})", name, src));
        }
        // Move WAV into stem dir first
        let wav_dest = stem_dir.join(format!("{}.wav", name));
        std::fs::rename(src_path, &wav_dest)
            .map_err(|e| format!("Failed to move {} stem: {}", name, e))?;

        // Encode WAV -> MP3 (device requires MP3; WAV is ~50 MB per stem)
        let mp3_dest = stem_dir.join(format!("{}.mp3", name));
        eprintln!("[splitter] Encoding {} to MP3...", name);
        wav_to_mp3(
            wav_dest.to_str().unwrap(),
            mp3_dest.to_str().unwrap(),
        )?;
        // Remove the intermediate WAV to save disk space
        let _ = std::fs::remove_file(&wav_dest);

        stem_files.push(mp3_dest.to_string_lossy().to_string());
    }

    Ok(SplitResult {
        output_dir: stem_dir.to_string_lossy().to_string(),
        track_name,
        stems: stem_files,
    })
}