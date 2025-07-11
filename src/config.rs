use serde::{Deserialize, Serialize};
use serde_yaml;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub program_name: String,
    pub program_path: PathBuf,
    pub update: Update,
    pub run: Run,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Update {
    /// in seconds
    pub interval: u32,
    /// list of bash commands; each String must be runnable in bash
    pub commands: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    /// list of bash commands; each String must be runnable in bash
    pub commands: Vec<String>,
}

pub fn parse_config(path: &PathBuf) -> Config {
    // Read the entire file to a string
    let contents = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read config file {:?}: {}", path, e));

    // Parse the YAML into your Config struct
    serde_yaml::from_str(&contents)
        .unwrap_or_else(|e| panic!("Failed to parse YAML in {:?}: {}", path, e))
}
