use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct DatabaseSettings {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct NntpSettings {
    pub server: String,
    pub port: u16,
    pub groups: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct AiSettings {
    pub provider: String,
    pub model: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, PartialEq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum IngestionMode {
    Nntp,
    LocalArchive,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct ArchiveSettings {
    pub path: String,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct IngestorSettings {
    pub mode: IngestionMode,
    pub archive: Option<ArchiveSettings>,
}

#[derive(Debug, Deserialize, Clone)]
#[allow(unused)]
pub struct Settings {
    pub database: DatabaseSettings,
    pub nntp: NntpSettings,
    pub ai: AiSettings,
    pub server: ServerSettings,
    pub ingestion: IngestorSettings,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let s = Config::builder()
            // Start with default settings
            .add_source(File::with_name("Settings"))
            // Add settings from environment variables (with a prefix of SASHIKO)
            // e.g. SASHIKO_SERVER__PORT=8081 would set the server port
            .add_source(Environment::with_prefix("SASHIKO").separator("__"))
            .build()?;

        s.try_deserialize()
    }
}
