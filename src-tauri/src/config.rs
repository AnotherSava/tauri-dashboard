use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub server_port: u16,
    pub always_on_top: bool,
    pub save_window_position: bool,
    pub window_position: Option<WindowPosition>,
    pub context_window_tokens: HashMap<String, u64>,
    pub context_bar_thresholds: Vec<Threshold>,
    pub benign_closers: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct WindowPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Threshold {
    pub percent: f32,
    pub color: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_port: 9077,
            always_on_top: true,
            save_window_position: false,
            window_position: None,
            context_window_tokens: [
                ("claude-opus-4-7".to_string(), 200_000),
                ("claude-sonnet-4-6".to_string(), 200_000),
                ("claude-haiku-4-5".to_string(), 200_000),
            ]
            .into_iter()
            .collect(),
            context_bar_thresholds: vec![
                Threshold { percent: 0.0, color: "#3a7c4a".into() },
                Threshold { percent: 60.0, color: "#c6a03c".into() },
                Threshold { percent: 85.0, color: "#c64a4a".into() },
            ],
            benign_closers: vec!["What's next?".into(), "Anything else?".into()],
        }
    }
}

impl Config {
    pub fn load_or_default(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                eprintln!("[config] failed to parse {path:?}: {e}; using defaults");
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .unwrap_or_else(|_| "{}".to_string());
        std::fs::write(path, json)
    }
}

pub struct ConfigState {
    pub config: Mutex<Config>,
    pub path: PathBuf,
}

impl ConfigState {
    pub fn new(path: PathBuf) -> Self {
        let config = Config::load_or_default(&path);
        Self {
            config: Mutex::new(config),
            path,
        }
    }

    pub fn snapshot(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Config) -> R,
    {
        let mut guard = self.config.lock().unwrap();
        f(&mut guard)
    }

    pub fn save_to_disk(&self) -> std::io::Result<()> {
        let snapshot = self.config.lock().unwrap().clone();
        snapshot.save(&self.path)
    }
}
