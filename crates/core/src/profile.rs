use anyhow::{Context, Result};
use directories::ProjectDirs;
use kbdsplit_shared::{ControllerSlot, DeviceId, KeyBinding};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::mapping::default_bindings;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub active_profile: String,
    pub profiles: BTreeMap<String, Profile>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let profile = Profile::default();
        let efootball = Profile {
            name: "eFootball".to_owned(),
            slots: ControllerSlot::ALL
                .into_iter()
                .map(|slot| {
                    (
                        slot,
                        SlotProfile {
                            device_id: None,
                            locked: false,
                            bindings: default_bindings(),
                        },
                    )
                })
                .collect(),
        };
        Self {
            active_profile: profile.name.clone(),
            profiles: BTreeMap::from([
                (profile.name.clone(), profile),
                (efootball.name.clone(), efootball),
            ]),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub slots: BTreeMap<ControllerSlot, SlotProfile>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "Default".to_owned(),
            slots: ControllerSlot::ALL
                .into_iter()
                .map(|slot| {
                    (
                        slot,
                        SlotProfile {
                            device_id: None,
                            locked: false,
                            bindings: default_bindings(),
                        },
                    )
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotProfile {
    pub device_id: Option<DeviceId>,
    pub locked: bool,
    pub bindings: Vec<KeyBinding>,
}

pub fn config_dir() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("dev", "kbdsplit", "kbdsplit")
        .context("could not locate user configuration directory")?;
    Ok(dirs.config_dir().to_path_buf())
}

pub struct ProfileStore {
    base: PathBuf,
}

impl ProfileStore {
    pub fn new() -> Result<Self> {
        Ok(Self { base: config_dir()? })
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self { base: path }
    }

    pub fn load(&self) -> Result<AppConfig> {
        let conf_path = self.base.join("config.toml");
        let profiles_dir = self.base.join("profiles");

        let active_profile = if conf_path.exists() {
            match fs::read_to_string(&conf_path) {
                Ok(text) => match toml::from_str::<serde_json::Value>(&text) {
                    Ok(cfg) => cfg
                        .get("active_profile")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Default")
                        .to_owned(),
                    Err(err) => {
                        tracing::warn!("failed to parse config.toml, using defaults: {err}");
                        "Default".to_owned()
                    }
                },
                Err(err) => {
                    tracing::warn!("failed to read config.toml, using defaults: {err}");
                    "Default".to_owned()
                }
            }
        } else {
            "Default".to_owned()
        };

        let mut profiles: BTreeMap<String, Profile> = BTreeMap::new();
        if profiles_dir.exists() {
            let mut entries: Vec<_> = fs::read_dir(&profiles_dir)
                .with_context(|| format!("failed to read {}", profiles_dir.display()))?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "toml"))
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in &entries {
                let text = fs::read_to_string(entry.path())
                    .with_context(|| format!("failed to read {}", entry.path().display()))?;
                match toml::from_str::<Profile>(&text) {
                    Ok(profile) => {
                        profiles.insert(profile.name.clone(), profile);
                    }
                    Err(err) => {
                        tracing::warn!("skipping {}: {err}", entry.path().display());
                    }
                }
            }
        }

        // If no profiles exist, create defaults
        if profiles.is_empty() {
            for p in [Profile::default(), Profile {
                name: "eFootball".to_owned(),
                slots: ControllerSlot::ALL
                    .into_iter()
                    .map(|slot| (slot, SlotProfile {
                        device_id: None,
                        locked: false,
                        bindings: default_bindings(),
                    }))
                    .collect(),
            }] {
                let name = p.name.clone();
                profiles.insert(name.clone(), p);
            }
        }

        let active = if profiles.contains_key(&active_profile) {
            active_profile
        } else {
            profiles.keys().next().unwrap().clone()
        };

        Ok(AppConfig {
            active_profile: active,
            profiles,
        })
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        let profiles_dir = self.base.join("profiles");
        fs::create_dir_all(&profiles_dir)
            .with_context(|| format!("failed to create {}", profiles_dir.display()))?;

        // Write config.toml
        let conf_path = self.base.join("config.toml");
        let text = toml::to_string_pretty(&serde_json::json!({
            "active_profile": config.active_profile
        })).context("failed to serialize config")?;
        fs::write(&conf_path, text)
            .with_context(|| format!("failed to write {}", conf_path.display()))?;

        // Write each profile to its own file
        for (name, profile) in &config.profiles {
            let file_name = format!("{name}.toml");
            let path = profiles_dir.join(&file_name);
            let text = toml::to_string_pretty(profile)
                .with_context(|| format!("failed to serialize profile {name}"))?;
            fs::write(&path, text)
                .with_context(|| format!("failed to write {}", path.display()))?;
        }

        // Remove stale profile files that are no longer in config
        if profiles_dir.exists() {
            for entry in fs::read_dir(&profiles_dir)
                .with_context(|| format!("failed to read {}", profiles_dir.display()))? {
                let entry = entry?;
                if entry.path().extension().is_some_and(|ext| ext == "toml")
                    && let Some(stem) = entry.path().file_stem().and_then(|s| s.to_str())
                    && !config.profiles.contains_key(stem)
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }

        Ok(())
    }

    pub fn profile_path(&self, name: &str) -> PathBuf {
        self.base.join("profiles").join(format!("{name}.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips() {
        let config = AppConfig::default();
        let text = toml::to_string(&config).unwrap();
        let decoded: AppConfig = toml::from_str(&text).unwrap();
        assert_eq!(decoded.active_profile, "Default");
        assert_eq!(decoded.profiles["Default"].slots.len(), 4);
        assert!(decoded.profiles.contains_key("eFootball"));
    }

    #[test]
    fn store_round_trips() {
        let dir = std::env::temp_dir().join("kbdsplit-test-store");
        let _ = fs::remove_dir_all(&dir);
        let store = ProfileStore::with_path(dir.clone());
        let config = store.load().unwrap();
        assert_eq!(config.profiles.len(), 2);
        assert!(config.profiles.contains_key("eFootball"));
        assert!(config.profiles.contains_key("Default"));
        store.save(&config).unwrap();
        // Verify per-file storage exists
        let profiles_dir = dir.join("profiles");
        assert!(profiles_dir.join("Default.toml").exists());
        assert!(profiles_dir.join("eFootball.toml").exists());
        assert!(dir.join("config.toml").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_loads_external_profile() {
        let dir = std::env::temp_dir().join("kbdsplit-test-external");
        let _ = fs::remove_dir_all(&dir);
        let profiles_dir = dir.join("profiles");
        fs::create_dir_all(&profiles_dir).unwrap();
        // Write an external profile file
        let external = Profile {
            name: "Custom".to_owned(),
            slots: ControllerSlot::ALL
                .into_iter()
                .map(|slot| (slot, SlotProfile {
                    device_id: None,
                    locked: false,
                    bindings: default_bindings(),
                }))
                .collect(),
        };
        let text = toml::to_string_pretty(&external).unwrap();
        fs::write(profiles_dir.join("Custom.toml"), text).unwrap();
        // Write config pointing to it
        fs::write(dir.join("config.toml"), r#"active_profile = "Custom""#).unwrap();
        let store = ProfileStore::with_path(dir.clone());
        let config = store.load().unwrap();
        assert_eq!(config.active_profile, "Custom");
        assert!(config.profiles.contains_key("Custom"));
        let _ = fs::remove_dir_all(&dir);
    }
}
