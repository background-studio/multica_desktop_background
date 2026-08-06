use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::models::{AppSettings, SettingsPatch};

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn write_json_transaction<T: Serialize>(file_path: &Path, value: &T) -> Result<(), String> {
    let directory = file_path
        .parent()
        .ok_or_else(|| "数据文件路径无效。".to_string())?;
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let temporary = file_path.with_extension(format!(
        "{}.{}.tmp",
        file_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("json"),
        Uuid::new_v4()
    ));
    let backup = file_path.with_extension(format!(
        "{}.bak",
        file_path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("json")
    ));
    let bytes = format!(
        "{}\n",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    let mut handle = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    handle
        .write_all(bytes.as_bytes())
        .and_then(|_| handle.sync_all())
        .map_err(|error| error.to_string())?;
    drop(handle);

    let _ = fs::remove_file(&backup);
    let had_original = file_path.exists();
    if had_original {
        fs::rename(file_path, &backup).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            error.to_string()
        })?;
    }
    if let Err(error) = fs::rename(&temporary, file_path) {
        let _ = fs::remove_file(&temporary);
        if had_original {
            let _ = fs::remove_file(file_path);
            let _ = fs::rename(&backup, file_path);
        }
        return Err(error.to_string());
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

pub struct SettingsStore {
    pub file_path: PathBuf,
    settings: AppSettings,
}

impl SettingsStore {
    pub fn load(data_directory: &Path) -> Result<Self, String> {
        let file_path = data_directory.join("settings.json");
        let settings = match fs::read_to_string(&file_path) {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(value) => AppSettings::normalize(&value),
                Err(_) => {
                    let invalid =
                        file_path.with_extension(format!("json.invalid-{}", timestamp_millis()));
                    let _ = fs::rename(&file_path, invalid);
                    AppSettings::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => AppSettings::default(),
            Err(_) => {
                let invalid =
                    file_path.with_extension(format!("json.invalid-{}", timestamp_millis()));
                let _ = fs::rename(&file_path, invalid);
                AppSettings::default()
            }
        };
        write_json_transaction(&file_path, &settings)?;
        Ok(Self {
            file_path,
            settings,
        })
    }

    pub fn value(&self) -> AppSettings {
        self.settings.clone()
    }

    pub fn save(&mut self, settings: AppSettings) -> Result<AppSettings, String> {
        self.settings = settings;
        write_json_transaction(&self.file_path, &self.settings)?;
        Ok(self.value())
    }

    pub fn patch(&mut self, patch: SettingsPatch) -> Result<AppSettings, String> {
        self.settings.apply_patch(patch);
        write_json_transaction(&self.file_path, &self.settings)?;
        Ok(self.value())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_utf8_settings_transactionally() {
        let root = std::env::temp_dir().join(format!("codex-settings-{}", Uuid::new_v4()));
        let mut store = SettingsStore::load(&root).expect("load settings");
        store.settings.active_media_id = Some("背景-一号".to_string());
        store.settings.display.opacity = 0.48;
        store.save(store.settings.clone()).expect("save settings");

        let reopened = SettingsStore::load(&root).expect("reopen settings");
        assert_eq!(
            reopened.settings.active_media_id.as_deref(),
            Some("背景-一号")
        );
        assert_eq!(reopened.settings.display.opacity, 0.48);
        let _ = fs::remove_dir_all(root);
    }
}
