use config::{ConfigError, Config, File};

// #[derive(Debug, Deserialize)]
// struct Location {
    // url: String,
// }

#[derive(Debug, Deserialize, Clone)]
pub struct Cowconfig {
    pub debug: bool,
    pub port: String,
    pub address: String,
    pub root_dir: String,
    // pub location: [Location],
}

impl Cowconfig {
    pub fn new() -> Result<Self, ConfigError> {
        let mut s = Config::new();
        s.merge(File::with_name("cow.toml"))?;

        s.try_into()
    }
}
