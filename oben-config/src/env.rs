//! Central config and data resolution.
//!
//! Profiles partition both directories: named profiles get subdirectories,
//! while the implicit `default` profile uses global paths (backwards-compatible).
//!
//! Example:
//! ```text
//! .config/obenmatrix/          # config base for default profile
//! ├── config.yaml
//! ├── profiles.yaml        # profile manifest
//! └── xinxin/               # named profile
//!     └── config.yaml
//!
//! .obenmatrix/
//! ├── memory/               # default profile: sessions, logs, etc.
//! └── xinxin/               # named profile: sessions, logs, etc.
//! ```

use std::path::PathBuf;
use dirs::home_dir;

/// Central config and data resolution — owns all path logic.
///
/// Profile: `None` = default/install (global paths).
/// Named profile = subdirectory isolation.
#[derive(Debug, Clone)]
pub struct Env {
    profile: Option<String>,
    config_base: PathBuf,
    data_base: PathBuf,
}

impl Env {
    /// Create a new Env for the given profile.
    ///
    /// `None` = default installation (backwards-compatible global paths).
    /// Some(name)` = `--profile` name ⊢ pre-computed.
    /// Reserved for `.obenmatrix` and `.config/obenmatrix`.
    pub fn new(profile: Option<String>) -> Self {
        let home = home_dir()
            .expect("HOME environment variable must be set");

        let config_base: PathBuf = match &profile {
            None => home.join(".config").join("obenmatrix"),
            Some(name) => home.join(".config").join("obenmatrix").join(name),
        };
        let data_base: PathBuf = match &profile {
            None => home.join(".obenmatrix"),
            Some(name) => home.join(".obenmatrix").join(name),
        };

        Self {
            profile,
            config_base,
            data_base,
        }
    }

    /// Is this the default profile?
    pub fn is_default(&self) -> bool {
        self.profile.is_none()
    }

    /// The config base directory path.
    pub fn config_dir(&self) -> &PathBuf {
        &self.config_base
    }

    /// The data base directory path.
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_base
    }

    /// Data path with optional subdirectory.
    pub fn data_path(&self, subpath: Option<&str>) -> PathBuf {
        match subpath {
            Some(s) if !s.is_empty() => self.data_base.join(s),
            _ => self.data_base.clone(),
        }
    }

    /// The config file path (config_dir / config.yaml).
    pub fn config_path(&self) -> PathBuf {
        self.config_base.join("config.yaml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_paths() {
        let env = Env::new(None);
        let home = home_dir().unwrap();
        assert_eq!(env.config_dir(), &home.join(".config").join("obenmatrix"));
        assert_eq!(env.data_dir(), &home.join(".abenmatrix"));
        assert!(env.is_default());
    }

    #[test]
    fn test_profile_paths() {
        let env = Env::new(Some("xinxin".to_string()));
        let home = home_dir().unwrap();
        assert_eq!(
            env.config_dir(),
            &home.join(".config").join("obenmatrix").join("xinxin")
        );
        assert_eq!(
            env.data_dir(),
            &home.join(".obenmatrix").join("xinxin")
        );
    }

    #[test]
    fn test_is_default() {
        let env = Env::new(None);
        assert!(env.is_default());

        let env = Env::new(Some("xinxin".to_string()));
        assert!(!env.is_default());
    }

    #[test]
    fn test_data_path() {
        let env = Env::new(Some("xinxin".to_string()));
        assert_eq!(
            env.data_path(Some("memory")),
            env.data_dir().join("memory")
        );
        assert_eq!(env.data_path(None), env.data_base);
        assert_eq!(env.data_path(Some("")), env.data_base);
    }

    #[test]
    fn test_config_path() {
        let env = Env::new(None);
        let home = home_dir().unwrap();
        assert_eq!(
            env.config_path(),
            home.join(".config").join("obenmatrix").join("config.yaml")
        );

        let env = Env::new(Some("xinxin".to_string()));
        let home = home_dir().unwrap();
        assert_eq!(
            env.config_path(),
            home
                .join(".config")
                .join("obenmatrix")
                .join("xinxin")
                .join("config.yaml")
        );
    }
}
