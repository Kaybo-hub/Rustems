use tauri::State;
use std::sync::Mutex;
use std::path::Path;

use crate::audio::{StemEngine, StemType};

// Error type for stem operations
#[derive(Debug, thiserror::Error)]
pub enum StemError {
    #[error("Invalid stem type: {0}")]
    InvalidStem(String),
    
    #[error("Audio error: {0}")]
    AudioError(String),
}

// Implement Serialize so Tauri can send the error string to JS
impl serde::Serialize for StemError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        serializer.serialize_str(self.to_string().as_str())
    }
}

fn stem_from_string(s: &str) -> Result<StemType, StemError> {
    match s.to_lowercase().as_str() {
        "drums" => Ok(StemType::Drums),
        "bass" => Ok(StemType::Bass),
        "melody" => Ok(StemType::Melody),
        "vocals" => Ok(StemType::Vocals),
        _ => Err(StemError::InvalidStem(s.to_string())),
    }
}

#[tauri::command]
pub fn load_song(folder: String, state: State<Mutex<StemEngine>>) -> Result<(), StemError> {
    println!("Loading song from folder: {}", folder);
    
    let mut engine = state.lock().unwrap();
    let stems = ["drums", "bass", "melody", "vocals"];
    let mut loaded_count = 0;

    for stem in stems {
        let extensions = ["wav", "mp3", "ogg", "flac"];
        let mut loaded = false;
        
        for ext in extensions {
            let path = format!("{}/{}.{}", folder, stem, ext);
            if Path::new(&path).exists() {
                println!("Found {} stem at: {}", stem, path);
                let stem_type = stem_from_string(stem)?;
                
                match engine.load_stem(stem_type, &path) {
                    Ok(_) => {
                        loaded = true;
                        loaded_count += 1;
                        println!("Successfully loaded {} stem", stem);
                        break;
                    }
                    Err(e) => {
                        println!("Failed to load {} stem from {}: {}", stem, path, e);
                    }
                }
            }
        }
        
        if !loaded {
            println!("Warning: Could not find any audio file for stem {}", stem);
        }
    }
    
    println!("Loaded {}/4 stems", loaded_count);
    
    if loaded_count == 0 {
        Err(StemError::AudioError("No stems could be loaded".to_string()))
    } else {
        Ok(())
    }
}

#[tauri::command]
pub fn play(state: State<Mutex<StemEngine>>) {
    println!("Play command received");
    state.lock().unwrap().play_all();
    println!("Play command executed");
}

#[tauri::command]
pub fn pause(state: State<Mutex<StemEngine>>) {
    println!("Pause command received");
    state.lock().unwrap().pause_all();
    println!("Pause command executed");
}

#[tauri::command]
pub fn set_stem_volume(
    stem: String, 
    volume: f64, 
    state: State<Mutex<StemEngine>>
) -> Result<(), StemError> {
    println!("Set volume for {} to {}", stem, volume);
    let stem_type = stem_from_string(&stem)?;
    
    let mut engine = state.lock().unwrap();
    engine.set_volume(stem_type, volume as f32);
    
    println!("Volume set successfully");
    Ok(())
}

#[tauri::command]
pub fn print_audio_status(state: State<Mutex<StemEngine>>) {
    state.lock().unwrap().print_status();
}