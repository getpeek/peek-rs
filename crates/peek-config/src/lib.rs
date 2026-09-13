//! Peek settings: `~/peek/settings.json`, its JSON schema, and the user's keymap overrides.
//!
//! Ported from the Tauri host's `config/mod.rs`. The serde shape is frozen: the TypeScript app
//! reads and writes the same file.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod keymap;
mod persistence;
mod theme_id;

pub use keymap::{KeymapError, gpui_keystroke};
pub use persistence::PersistenceMode;
pub use theme_id::ThemeId;

const SETTINGS_SCHEMA: &str = include_str!("./settings.schema.json");

fn default_schema_ref() -> String {
    "./settings.schema.json".to_string()
}

#[derive(Debug)]
pub enum ConfigError {
    NoHomeDirectory,
    ReadOnly,
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHomeDirectory => write!(formatter, "HOME is not set"),
            Self::ReadOnly => write!(formatter, "settings are read-only in this build"),
            Self::Io(error) => write!(formatter, "settings io error: {error}"),
            Self::Json(error) => write!(formatter, "settings json error: {error}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// `~/peek`: settings, schema and the `workspaces/` document tree.
///
/// # Errors
/// Returns an error when `HOME` is not set.
pub fn config_dir() -> Result<PathBuf, ConfigError> {
    let home = std::env::var("HOME").map_err(|_| ConfigError::NoHomeDirectory)?;
    Ok(Path::new(&home).join("peek"))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PeekConfig {
    #[serde(rename = "$schema", default = "default_schema_ref")]
    schema: String,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub theme: ThemeId,
    /// User keymap overrides (`key -> "Group::Variant"`). Stored as raw strings, not typed
    /// actions, so an unknown action name can't fail deserialization and reset the whole
    /// config — the command registry validates and merges these over the defaults.
    #[serde(default)]
    pub keymap: HashMap<String, String>,
    #[serde(default)]
    pub canvas: CanvasConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

impl Default for PeekConfig {
    fn default() -> Self {
        Self {
            schema: default_schema_ref(),
            workspaces: Vec::new(),
            ai: AiConfig::default(),
            name: None,
            theme: ThemeId::default(),
            keymap: HashMap::new(),
            canvas: CanvasConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

impl PeekConfig {
    /// Loads `~/peek/settings.json`, falling back to defaults when the file is missing or
    /// unparsable (the Tauri app behaves the same way), and fills in the display name.
    #[must_use]
    pub fn get_or_default() -> Self {
        let loaded = config_dir()
            .ok()
            .map_or_else(Self::default, |dir| Self::load_from(&dir));
        loaded.with_default_name()
    }

    /// Loads `settings.json` from `dir`, falling back to defaults when missing or unparsable.
    #[must_use]
    pub fn load_from(dir: &Path) -> Self {
        let Ok(contents) = std::fs::read_to_string(dir.join("settings.json")) else {
            return Self::default();
        };
        serde_json::from_str(&contents).unwrap_or_else(|error| {
            log::warn!("peek: settings.json is unreadable, using defaults: {error}");
            Self::default()
        })
    }

    fn with_default_name(mut self) -> Self {
        if self.name.is_none() {
            self.name = Some(
                std::env::var("USER")
                    .or_else(|_| std::env::var("USERNAME"))
                    .unwrap_or_else(|_| "Anonymous".to_string()),
            );
        }
        self
    }

    /// Ensures `~/peek/` exists, that `settings.schema.json` is up to date and that
    /// `settings.json` exists. A no-op in [`PersistenceMode::ReadOnly`].
    ///
    /// # Errors
    /// Returns an error if the directory or its files cannot be created or written.
    pub fn ensure_initialized_on_disk(mode: PersistenceMode) -> Result<(), ConfigError> {
        if !mode.can_write() {
            log::info!("peek: read-only mode, skipping settings initialisation");
            return Ok(());
        }
        let dir = config_dir()?;
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("settings.schema.json"), SETTINGS_SCHEMA)?;

        let settings_path = dir.join("settings.json");
        if !settings_path.exists() {
            let serialized = serde_json::to_string_pretty(&Self::default())?;
            std::fs::write(&settings_path, serialized)?;
        }
        Ok(())
    }

    /// Writes the whole config back to `~/peek/settings.json`.
    ///
    /// # Errors
    /// Returns [`ConfigError::ReadOnly`] in read-only mode, or an io/serde error.
    pub fn save_to_disk(&self, mode: PersistenceMode) -> Result<(), ConfigError> {
        if !mode.can_write() {
            return Err(ConfigError::ReadOnly);
        }
        let dir = config_dir()?;
        std::fs::create_dir_all(&dir)?;
        let serialized = serde_json::to_string_pretty(self)?;
        std::fs::write(dir.join("settings.json"), serialized)?;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CanvasConfig {
    /// Region grouping and wayfinding (beacons, edge peekers) on the canvas.
    #[serde(default = "CanvasConfig::default_enable_regions")]
    pub enable_regions: bool,
    /// Overview minimap in the bottom-right corner of the canvas.
    #[serde(default = "CanvasConfig::default_minimap")]
    pub minimap: Visibility,
}

impl CanvasConfig {
    fn default_enable_regions() -> bool {
        true
    }

    // Opt-in: `Visibility`'s own default is `Show`, which is wrong here.
    fn default_minimap() -> Visibility {
        Visibility::Hide
    }
}

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            enable_regions: Self::default_enable_regions(),
            minimap: Self::default_minimap(),
        }
    }
}

/// How pages are surfaced for navigation.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PageDisplay {
    /// A horizontal tab strip in the titlebar.
    #[default]
    Tabs,
    /// A single pill in the titlebar that opens a keyboard-navigable list.
    List,
}

/// Whether a piece of UI chrome is rendered.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    #[default]
    Show,
    Hide,
}

impl Visibility {
    #[must_use]
    pub fn is_shown(self) -> bool {
        self == Self::Show
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PagesConfig {
    #[serde(default)]
    pub show_as: PageDisplay,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TitlebarConfig {
    #[serde(default)]
    pub command_palette_button: Visibility,
    #[serde(default)]
    pub collaboration_button: Visibility,
    #[serde(default)]
    pub live_query_count: Visibility,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub pages: PagesConfig,
    #[serde(default)]
    pub titlebar: TitlebarConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpConfig {
    /// Run the MCP server at startup. Changing this takes effect on restart.
    #[serde(default)]
    pub enable: bool,
    #[serde(default = "McpConfig::default_port")]
    pub port: u16,
}

impl McpConfig {
    fn default_port() -> u16 {
        13315
    }
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enable: false,
            port: Self::default_port(),
        }
    }
}

/// Which backend answers the built-in agent. Each node picks its own provider; this is only
/// the default for new nodes.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    // `alias` keeps configs written with the earlier `"api"` value loading.
    #[default]
    #[serde(alias = "api")]
    Ollama,
    Acp,
}

/// The OpenAI/Ollama-compatible completion endpoint. Its presence under `ai` is what enables
/// the AI features — absent means they're unavailable.
#[derive(Debug, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "OllamaConfig::default_model")]
    pub model: String,
    #[serde(default = "OllamaConfig::default_url")]
    pub url: String,
}

impl OllamaConfig {
    fn default_model() -> String {
        "gemma4:e2b".to_string()
    }

    fn default_url() -> String {
        "http://localhost:11434".to_string()
    }
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            model: Self::default_model(),
            url: Self::default_url(),
        }
    }
}

/// How to spawn an ACP agent subprocess. The agent owns its own auth — pass credentials it
/// expects (e.g. `ANTHROPIC_API_KEY`) through `env`.
#[derive(Debug, Serialize, Deserialize)]
pub struct AcpConfig {
    #[serde(default = "AcpConfig::default_command")]
    pub command: String,
    #[serde(default = "AcpConfig::default_args")]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Session root handed to the agent at `session/new`. Falls back to `~/peek` when unset.
    #[serde(default)]
    pub cwd: Option<String>,
}

impl AcpConfig {
    fn default_command() -> String {
        "npx".to_string()
    }

    fn default_args() -> Vec<String> {
        vec![
            "-y".to_string(),
            "@agentclientprotocol/claude-agent-acp".to_string(),
        ]
    }
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            command: Self::default_command(),
            args: Self::default_args(),
            env: HashMap::new(),
            cwd: None,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AiConfig {
    /// Default backend for new Agent nodes; `alias` migrates the older flat `provider` key.
    #[serde(default, alias = "provider")]
    pub default_provider: AiProvider,
    /// Present iff the ollama/OpenAI-compatible backend is configured.
    #[serde(default)]
    pub ollama: Option<OllamaConfig>,
    /// Ask the AI for a short label when a query node is executed (needs `ollama`).
    #[serde(default)]
    pub automatically_label_queries: bool,
    /// Present iff an external ACP agent is configured.
    #[serde(default)]
    pub acp: Option<AcpConfig>,
    #[serde(default)]
    pub mcp: McpConfig,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Workspace {
    pub name: String,
    pub connections: Vec<DatabaseConnection>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DatabaseConnection {
    pub name: String,
    pub color: String,
    pub url: String,
    #[serde(default)]
    pub ssh_tunnel: Option<SshTunnelConfig>,
}

impl DatabaseConnection {
    /// The connection's tint as sRGB bytes, or `None` when the string is not one of the two forms
    /// the app writes. `~/peek/settings.json` holds both (`"#5584E8"`, `"hsl(20deg, 80%, 60%)"`),
    /// because the TypeScript colour picker emits hex and the seeded defaults are `hsl`.
    #[must_use]
    pub fn rgb(&self) -> Option<(u8, u8, u8)> {
        let color = self.color.trim();
        color
            .strip_prefix('#')
            .and_then(parse_hex)
            .or_else(|| parse_hsl(color))
    }
}

impl DatabaseConnection {
    /// `user@host` from the connection URL, the reference's `parseConnectionUrl`. Deliberately
    /// narrow: it must never surface a password, so only the part of the credentials before the
    /// first `:` is kept.
    #[must_use]
    pub fn origin(&self) -> Option<String> {
        let authority = self.url.split("://").nth(1)?.split(['/', '?']).next()?;
        let (credentials, host) = authority.rsplit_once('@')?;
        let user = credentials.split(':').next().unwrap_or(credentials);
        let host = host.split(':').next().unwrap_or(host);
        (!user.is_empty() && !host.is_empty()).then(|| format!("{user}@{host}"))
    }
}

fn parse_hex(digits: &str) -> Option<(u8, u8, u8)> {
    let expand = |digit: u8| digit * 17;
    match digits.len() {
        3 => {
            let mut nibbles = digits.chars().map(|digit| digit.to_digit(16));
            Some((
                expand(u8::try_from(nibbles.next()??).ok()?),
                expand(u8::try_from(nibbles.next()??).ok()?),
                expand(u8::try_from(nibbles.next()??).ok()?),
            ))
        }
        // A trailing alpha pair is dropped: the tint is used at fixed opacities.
        6 | 8 => Some((
            u8::from_str_radix(&digits[0..2], 16).ok()?,
            u8::from_str_radix(&digits[2..4], 16).ok()?,
            u8::from_str_radix(&digits[4..6], 16).ok()?,
        )),
        _ => None,
    }
}

/// `hsl(H[deg], S%, L%)` — the only functional form the app writes. Commas or spaces separate the
/// components, as CSS allows both.
fn parse_hsl(color: &str) -> Option<(u8, u8, u8)> {
    let body = color
        .strip_prefix("hsl(")
        .or_else(|| color.strip_prefix("hsla("))?
        .strip_suffix(')')?;
    let mut parts = body
        .split([',', ' ', '/'])
        .filter(|part| !part.is_empty())
        .map(str::trim);
    let hue: f32 = strip_unit(parts.next()?, "deg").parse().ok()?;
    let saturation: f32 = strip_unit(parts.next()?, "%").parse().ok()?;
    let lightness: f32 = strip_unit(parts.next()?, "%").parse().ok()?;
    Some(hsl_to_rgb(
        hue.rem_euclid(360.0),
        (saturation / 100.0).clamp(0.0, 1.0),
        (lightness / 100.0).clamp(0.0, 1.0),
    ))
}

fn strip_unit<'a>(value: &'a str, unit: &str) -> &'a str {
    value.strip_suffix(unit).unwrap_or(value)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the components are clamped to 0..=1 before scaling to a byte"
)]
fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (u8, u8, u8) {
    let chroma = (1.0 - (2.0f32.mul_add(lightness, -1.0)).abs()) * saturation;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (red, green, blue) = match sector as u8 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let base = chroma.mul_add(-0.5, lightness);
    let byte = |channel: f32| ((channel + base) * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(red), byte(green), byte(blue))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SshTunnelConfig {
    pub ssh_host: String,
    pub ssh_user: String,
    /// Path to the SSH private key (`.pem` file).
    pub key_path: PathBuf,
    #[serde(default = "SshTunnelConfig::default_ssh_port")]
    pub ssh_port: u16,
    /// Local port for the tunnel. `0` lets the OS pick a free port.
    #[serde(default = "SshTunnelConfig::default_local_port")]
    pub local_port: u16,
}

impl SshTunnelConfig {
    fn default_ssh_port() -> u16 {
        22
    }

    fn default_local_port() -> u16 {
        15432
    }
}

#[cfg(test)]
mod tests {
    use super::DatabaseConnection;

    fn tinted(color: &str) -> Option<(u8, u8, u8)> {
        DatabaseConnection {
            color: color.to_string(),
            ..DatabaseConnection::default()
        }
        .rgb()
    }

    #[test]
    fn parses_the_hex_the_colour_picker_writes() {
        assert_eq!(tinted("#5584E8"), Some((0x55, 0x84, 0xE8)));
        assert_eq!(tinted("#5584e8"), Some((0x55, 0x84, 0xE8)));
        assert_eq!(tinted(" #fff "), Some((255, 255, 255)));
        assert_eq!(tinted("#5584E8FF"), Some((0x55, 0x84, 0xE8)));
    }

    #[test]
    fn parses_the_hsl_the_seeded_defaults_use() {
        // The two `hsl` values in a real `settings.json`.
        assert_eq!(tinted("hsl(60deg, 70%, 55%)"), Some((221, 221, 60)));
        assert_eq!(tinted("hsl(20deg, 80%, 60%)"), Some((235, 126, 71)));
        // Commas are optional in CSS, and so is the `deg`.
        assert_eq!(tinted("hsl(20 80% 60%)"), tinted("hsl(20deg, 80%, 60%)"));
    }

    #[test]
    fn grey_and_black_survive_the_hue_sectors() {
        assert_eq!(tinted("hsl(0deg, 0%, 0%)"), Some((0, 0, 0)));
        assert_eq!(tinted("hsl(0deg, 0%, 100%)"), Some((255, 255, 255)));
        // Hue 360 wraps onto the first sector rather than falling off the end.
        assert_eq!(
            tinted("hsl(360deg, 80%, 60%)"),
            tinted("hsl(0deg, 80%, 60%)")
        );
    }

    fn at(url: &str) -> Option<String> {
        DatabaseConnection {
            url: url.to_string(),
            ..DatabaseConnection::default()
        }
        .origin()
    }

    #[test]
    fn the_origin_names_the_user_and_host_without_the_password() {
        assert_eq!(
            at("postgres://dbuser:hunter2@db.example.com:5432/app"),
            Some("dbuser@db.example.com".to_string()),
        );
        assert_eq!(
            at(
                "postgresql://neondb_owner:pw@ep-1.eu-central-1.aws.neon.tech/neondb?sslmode=require"
            ),
            Some("neondb_owner@ep-1.eu-central-1.aws.neon.tech".to_string()),
        );
    }

    #[test]
    fn a_url_with_no_credentials_has_no_origin_to_show() {
        assert_eq!(at("postgres://localhost:5432/app"), None);
        assert_eq!(at("not a url"), None);
        assert_eq!(at(""), None);
    }

    #[test]
    fn an_unparsable_tint_is_none_rather_than_a_guess() {
        assert_eq!(tinted(""), None);
        assert_eq!(tinted("rebeccapurple"), None);
        assert_eq!(tinted("#12345"), None);
        assert_eq!(tinted("hsl(20deg, 80%)"), None);
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn parses_a_minimal_settings_file() {
        let config: PeekConfig = serde_json::from_str(
            r#"{ "theme": "paper", "keymap": { "meta-1": "Tool::Query" }, "workspaces": [] }"#,
        )
        .unwrap();
        assert_eq!(config.theme, ThemeId::Paper);
        assert_eq!(config.keymap["meta-1"], "Tool::Query");
        assert!(config.canvas.enable_regions);
        assert_eq!(config.canvas.minimap, Visibility::Hide);
    }

    #[test]
    fn unknown_theme_falls_back_to_defaults_at_the_file_level() {
        // Matches the Tauri host: an unparsable file yields the default config.
        let parsed: Result<PeekConfig, _> = serde_json::from_str(r#"{ "theme": "neon" }"#);
        assert!(parsed.is_err());
    }
}
