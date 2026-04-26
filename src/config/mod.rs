mod general;
mod logging;
mod platform;
mod proxy;

pub use general::GeneralConfig;
pub use logging::LoggingConfig;
pub use platform::{Platform, PlatformConfig};
pub use proxy::ProxyConfig;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub logging: LoggingConfig,
    pub platform: PlatformConfig,
    pub proxy: ProxyConfig,
}

enum ResolvedPath {
    Existing(PathBuf),
    Missing(PathBuf),
}

fn resolve_config_path(cli_path: Option<&Path>) -> ResolvedPath {
    if let Some(p) = cli_path {
        if p.exists() {
            return ResolvedPath::Existing(p.to_path_buf());
        }
        return ResolvedPath::Missing(p.to_path_buf());
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidate = exe_dir.join("configs").join("config.toml");
            if candidate.exists() {
                return ResolvedPath::Existing(candidate);
            }
        }
    }

    let cwd_candidate = PathBuf::from("configs.toml");
    if cwd_candidate.exists() {
        return ResolvedPath::Existing(cwd_candidate);
    }

    let target = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("configs").join("config.toml")))
        .unwrap_or(cwd_candidate);
    ResolvedPath::Missing(target)
}

fn write_default_config(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let defaults = AppConfig::default();
    let body = toml::to_string_pretty(&defaults)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, body)
}

pub fn load_config(cli_path: Option<&Path>) -> AppConfig {
    let path = match resolve_config_path(cli_path) {
        ResolvedPath::Existing(p) => p,
        ResolvedPath::Missing(target) => {
            eprintln!(
                "No config file found, writing defaults to: {}",
                target.display()
            );
            match write_default_config(&target) {
                Ok(()) => target,
                Err(e) => {
                    eprintln!("Failed to write default config: {e}, using in-memory defaults");
                    return AppConfig::default();
                }
            }
        }
    };

    eprintln!("Loading config from: {}", path.display());

    match std::fs::read_to_string(&path) {
        Ok(content) => match toml::from_str::<AppConfig>(&content) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Failed to parse config: {e}, using defaults");
                AppConfig::default()
            }
        },
        Err(e) => {
            eprintln!("Failed to read config: {e}, using defaults");
            AppConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        d.push(format!("akagi-cfg-{tag}-{pid}-{nanos}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writes_defaults_when_cli_path_missing() {
        let dir = temp_dir("cli-missing");
        let target = dir.join("nested").join("config.toml");
        assert!(!target.exists());

        let cfg = load_config(Some(&target));

        assert!(target.exists(), "default config file should be created");
        let body = std::fs::read_to_string(&target).unwrap();
        let round_trip: AppConfig = toml::from_str(&body).unwrap();
        assert_eq!(round_trip.general.language, cfg.general.language);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reuses_existing_cli_path() {
        let dir = temp_dir("cli-existing");
        let target = dir.join("config.toml");
        std::fs::write(&target, "[general]\nlanguage = \"jp\"\n").unwrap();

        let cfg = load_config(Some(&target));
        assert_eq!(cfg.general.language, "jp");

        std::fs::remove_dir_all(&dir).ok();
    }
}
