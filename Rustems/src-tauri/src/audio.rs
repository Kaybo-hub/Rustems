use kira::{
    AudioManager, AudioManagerSettings,
    sound::static_sound::{StaticSoundData, StaticSoundHandle},
    DefaultBackend, Tween,
    Decibels,
};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub enum StemType {
    Drums,
    Bass,
    Melody,
    Vocals,
}

pub struct StemEngine {
    manager: AudioManager<DefaultBackend>,
    handles: HashMap<StemType, StaticSoundHandle>,
    paths: HashMap<StemType, String>,
}

impl StemEngine {
    pub fn new() -> Self {
        let manager = 
            AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
                .expect("Failed to create audio manager");

        Self {
            manager,
            handles: HashMap::new(),
            paths: HashMap::new(),
        }
    }

    pub fn load_stem(&mut self, stem: StemType, path: &str) -> Result<(), String> {
        println!("Loading stem {:?} from {}", stem, path);
        
        // Stop existing handle
        if let Some(mut old) = self.handles.remove(&stem) {
            println!("Stopping existing handle for {:?}", stem);
            old.stop(Tween::default());
        }

        self.paths.insert(stem, path.to_string());

        // Try to load the sound file
        let sound = match StaticSoundData::from_file(path) {
            Ok(s) => {
                println!("Successfully loaded audio file for {:?}", stem);
                s
            },
            Err(e) => {
                let error = format!("Failed to load audio file {}: {}", path, e);
                println!("{}", error);
                return Err(error);
            }
        };
        
        // Try to play the sound
        let mut handle = match self.manager.play(sound) {
            Ok(h) => {
                println!("Successfully created sound handle for {:?}", stem);
                h
            },
            Err(e) => {
                let error = format!("Failed to play sound for {:?}: {}", stem, e);
                println!("{}", error);
                return Err(error);
            }
        };
        
        // Pause it immediately (starts paused)
        handle.pause(Tween::default());
        println!("Paused {:?} stem", stem);
        
        self.handles.insert(stem, handle);
        println!("{:?} stem loaded and ready", stem);
        
        Ok(())
    }

    pub fn play_all(&mut self) {
        println!("Playing all stems");
        for (stem, h) in self.handles.iter_mut() {
            println!("Playing {:?}", stem);
            h.resume(Tween::default());
        }
    }

    pub fn pause_all(&mut self) {
        println!("Pausing all stems");
        for (stem, h) in self.handles.iter_mut() {
            println!("Pausing {:?}", stem);
            h.pause(Tween::default());
        }
    }

    pub fn set_volume(&mut self, stem: StemType, volume: f32) {
        if let Some(h) = self.handles.get_mut(&stem) {
            // Convert linear 0.0-1.0 to decibels
            // Silence threshold at -60dB (Decibels::SILENCE)
            let db = if volume <= 0.0 {
                Decibels::SILENCE
            } else {
                Decibels(20.0 * volume.log10())
            };
            
            h.set_volume(
                db,
                Tween {
                    duration: Duration::from_millis(50),
                    ..Default::default()
                },
            );
        }
    }

    pub fn print_status(&self) {
        println!("=== Audio Status ===");
        for (stem, _handle) in self.handles.iter() {
            println!("{:?}:", stem);
            println!("  Handle exists");
            if let Some(path) = self.paths.get(stem) {
                println!("  Path: {}", path);
            }
        }
    }
}