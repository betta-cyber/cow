use config::{ConfigError, Config, File};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct Server {
    pub location: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Cowconfig {
    pub debug: bool,
    pub port: String,
    pub address: String,
    pub root_dir: String,
    pub server: Vec<Server>,
}

impl Cowconfig {
    pub fn new(path: &str) -> Result<Self, ConfigError> {
        let mut s = Config::new();
        s.merge(File::with_name(path))?;

        s.try_into()
    }
}
