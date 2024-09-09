use std::{
    fs::{self},
    path::PathBuf,
};

use projectm::core::ProjectM;
use rand::Rng;
use walkdir::WalkDir;

#[derive(Default)]
pub struct Playlist {
    presets: Vec<(String, String)>,
    current_index: usize,
}

impl Playlist {
    pub fn add_dir(&mut self, dir: PathBuf) {
        assert!(dir.is_dir());

        for entry in WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_entry(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|s| !s.starts_with("."))
                    .unwrap_or(false)
            })
            .filter_map(Result::ok)
        {
            println!("reading {:?}", entry.path());
            if let Ok(contents) = fs::read_to_string(entry.path()) {
                self.presets
                    .push((entry.file_name().to_string_lossy().into_owned(), contents))
            } else {
                println!("not valid UTF-8! skipping...")
            }
        }
    }

    pub fn play_random(&mut self, pm: &ProjectM, smooth: bool) {
        self.current_index = rand::thread_rng().gen_range(0..self.presets.len());

        self.load_current_preset(pm, smooth);
    }

    fn load_current_preset(&mut self, pm: &ProjectM, smooth: bool) {
        let preset = &self.presets[self.current_index];
        println!("loading {}...", preset.0);
        pm.load_preset_data(&preset.1, smooth);
        println!("done!");
    }

    pub fn play_index(&mut self, pm: &ProjectM, index: usize, smooth: bool) {
        self.current_index = index;

        self.load_current_preset(pm, smooth);
    }

    pub fn presets(&self) -> impl Iterator<Item = &str> + '_ {
        self.presets.iter().map(|(name, _)| name.as_str())
    }

    pub fn current_index(&self) -> usize {
        self.current_index
    }
}
