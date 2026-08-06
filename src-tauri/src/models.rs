use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FitMode {
    #[default]
    Cover,
    Contain,
    Fill,
    Tile,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SlideshowOrder {
    #[default]
    Sequential,
    Random,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MediaOrigin {
    Local,
    Remote,
    Api,
    Folder,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub id: String,
    pub name: String,
    pub kind: MediaKind,
    pub origin: MediaOrigin,
    pub file_name: String,
    pub mime_type: String,
    pub byte_size: u64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// 文件夹源内扫描到的媒体数量；普通文件条目不使用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_count: Option<u64>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplaySettings {
    pub fit: FitMode,
    pub position_x: f64,
    pub position_y: f64,
    pub opacity: f64,
    pub blur: f64,
    pub scale: f64,
    pub overlay_color: String,
    pub overlay_opacity: f64,
    pub block_fill_opacity: f64,
    pub home_intensity: f64,
    pub task_intensity: f64,
    pub sidebar_opacity: f64,
    pub surface_opacity: f64,
    pub card_opacity: f64,
    pub composer_opacity: f64,
    pub menu_opacity: f64,
    pub terminal_opacity: f64,
    pub enabled_on_home: bool,
    pub enabled_on_tasks: bool,
    pub video_muted: bool,
    pub video_playback_rate: f64,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            fit: FitMode::Cover,
            position_x: 50.0,
            position_y: 50.0,
            opacity: 0.72,
            blur: 0.0,
            scale: 1.0,
            overlay_color: "#101416".to_string(),
            overlay_opacity: 0.12,
            block_fill_opacity: 0.55,
            home_intensity: 1.0,
            task_intensity: 0.32,
            sidebar_opacity: 0.18,
            surface_opacity: 0.12,
            card_opacity: 0.35,
            composer_opacity: 0.88,
            menu_opacity: 0.9,
            terminal_opacity: 0.9,
            enabled_on_home: true,
            enabled_on_tasks: true,
            video_muted: true,
            video_playback_rate: 1.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideshowSettings {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub order: SlideshowOrder,
}

impl Default for SlideshowSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_seconds: 300,
            order: SlideshowOrder::Sequential,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorSettings {
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub auto_start_with_windows: bool,
    pub launch_multica_on_apply: bool,
}

impl Default for BehaviorSettings {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            start_minimized: false,
            auto_start_with_windows: false,
            launch_multica_on_apply: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u8,
    pub active_media_id: Option<String>,
    pub playlist_ids: Vec<String>,
    pub display: DisplaySettings,
    pub slideshow: SlideshowSettings,
    pub behavior: BehaviorSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            active_media_id: None,
            playlist_ids: Vec::new(),
            display: DisplaySettings::default(),
            slideshow: SlideshowSettings::default(),
            behavior: BehaviorSettings::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayPatch {
    pub fit: Option<FitMode>,
    pub position_x: Option<f64>,
    pub position_y: Option<f64>,
    pub opacity: Option<f64>,
    pub blur: Option<f64>,
    pub scale: Option<f64>,
    pub overlay_color: Option<String>,
    pub overlay_opacity: Option<f64>,
    pub block_fill_opacity: Option<f64>,
    pub home_intensity: Option<f64>,
    pub task_intensity: Option<f64>,
    pub sidebar_opacity: Option<f64>,
    pub surface_opacity: Option<f64>,
    pub card_opacity: Option<f64>,
    pub composer_opacity: Option<f64>,
    pub menu_opacity: Option<f64>,
    pub terminal_opacity: Option<f64>,
    pub enabled_on_home: Option<bool>,
    pub enabled_on_tasks: Option<bool>,
    pub video_muted: Option<bool>,
    pub video_playback_rate: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideshowPatch {
    pub enabled: Option<bool>,
    pub interval_seconds: Option<f64>,
    pub order: Option<SlideshowOrder>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorPatch {
    pub close_to_tray: Option<bool>,
    pub start_minimized: Option<bool>,
    pub auto_start_with_windows: Option<bool>,
    pub launch_multica_on_apply: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub active_media_id: Option<Option<String>>,
    pub playlist_ids: Option<Vec<String>>,
    pub display: Option<DisplayPatch>,
    pub slideshow: Option<SlideshowPatch>,
    pub behavior: Option<BehaviorPatch>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    #[serde(default)]
    pub dynamic: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRequest {
    #[allow(dead_code)]
    pub restart_existing: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub phase: String,
    pub message: String,
    pub active_targets: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multica_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl Default for RuntimeStatus {
    fn default() -> Self {
        Self {
            phase: "idle".to_string(),
            message: "尚未连接 Multica".to_string(),
            active_targets: 0,
            multica_version: None,
            last_error: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub settings: AppSettings,
    pub library: Vec<MediaItem>,
    pub runtime: RuntimeStatus,
    pub data_directory: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ImportResult {
    pub added: Vec<MediaItem>,
    pub skipped: Vec<SkippedImport>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkippedImport {
    pub path: String,
    pub reason: String,
}

fn object<'a>(value: &'a Value, key: &str) -> Option<&'a serde_json::Map<String, Value>> {
    value.get(key)?.as_object()
}

fn number(map: Option<&serde_json::Map<String, Value>>, key: &str, fallback: f64) -> f64 {
    map.and_then(|value| value.get(key))
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
        .filter(|value| value.is_finite())
        .unwrap_or(fallback)
}

fn boolean(map: Option<&serde_json::Map<String, Value>>, key: &str, fallback: bool) -> bool {
    map.and_then(|value| value.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(fallback)
}

fn clamp(value: f64, minimum: f64, maximum: f64) -> f64 {
    value.clamp(minimum, maximum)
}

impl AppSettings {
    pub fn normalize(value: &Value) -> Self {
        let defaults = Self::default();
        let display = object(value, "display");
        let slideshow = object(value, "slideshow");
        let behavior = object(value, "behavior");

        let fit = display
            .and_then(|map| map.get("fit"))
            .and_then(Value::as_str)
            .and_then(|fit| match fit {
                "cover" => Some(FitMode::Cover),
                "contain" => Some(FitMode::Contain),
                "fill" => Some(FitMode::Fill),
                "tile" => Some(FitMode::Tile),
                _ => None,
            })
            .unwrap_or(defaults.display.fit);
        let order = slideshow
            .and_then(|map| map.get("order"))
            .and_then(Value::as_str)
            .and_then(|order| match order {
                "sequential" => Some(SlideshowOrder::Sequential),
                "random" => Some(SlideshowOrder::Random),
                _ => None,
            })
            .unwrap_or(defaults.slideshow.order);
        let overlay_color = display
            .and_then(|map| map.get("overlayColor"))
            .and_then(Value::as_str)
            .filter(|color| {
                color.len() == 7
                    && color.starts_with('#')
                    && color[1..]
                        .chars()
                        .all(|character| character.is_ascii_hexdigit())
            })
            .map(str::to_ascii_lowercase)
            .unwrap_or(defaults.display.overlay_color);
        let active_media_id = value
            .get("activeMediaId")
            .and_then(Value::as_str)
            .filter(|id| id.len() <= 120)
            .map(str::to_string);
        let mut playlist_ids = Vec::new();
        if let Some(values) = value.get("playlistIds").and_then(Value::as_array) {
            for id in values
                .iter()
                .filter_map(Value::as_str)
                .filter(|id| id.len() <= 120)
            {
                if !playlist_ids.iter().any(|existing| existing == id) {
                    playlist_ids.push(id.to_string());
                }
            }
        }

        Self {
            schema_version: 1,
            active_media_id,
            playlist_ids,
            display: DisplaySettings {
                fit,
                position_x: clamp(
                    number(display, "positionX", defaults.display.position_x),
                    0.0,
                    100.0,
                ),
                position_y: clamp(
                    number(display, "positionY", defaults.display.position_y),
                    0.0,
                    100.0,
                ),
                opacity: clamp(
                    number(display, "opacity", defaults.display.opacity),
                    0.0,
                    1.0,
                ),
                blur: clamp(number(display, "blur", defaults.display.blur), 0.0, 40.0),
                scale: clamp(number(display, "scale", defaults.display.scale), 1.0, 1.3),
                overlay_color,
                overlay_opacity: clamp(
                    number(display, "overlayOpacity", defaults.display.overlay_opacity),
                    0.0,
                    0.9,
                ),
                block_fill_opacity: clamp(
                    number(
                        display,
                        "blockFillOpacity",
                        defaults.display.block_fill_opacity,
                    ),
                    0.0,
                    1.0,
                ),
                home_intensity: clamp(
                    number(display, "homeIntensity", defaults.display.home_intensity),
                    0.0,
                    1.0,
                ),
                task_intensity: clamp(
                    number(display, "taskIntensity", defaults.display.task_intensity),
                    0.0,
                    1.0,
                ),
                sidebar_opacity: clamp(
                    number(display, "sidebarOpacity", defaults.display.sidebar_opacity),
                    0.0,
                    1.0,
                ),
                surface_opacity: clamp(
                    number(display, "surfaceOpacity", defaults.display.surface_opacity),
                    0.0,
                    1.0,
                ),
                card_opacity: clamp(
                    number(display, "cardOpacity", defaults.display.card_opacity),
                    0.0,
                    1.0,
                ),
                composer_opacity: clamp(
                    number(
                        display,
                        "composerOpacity",
                        defaults.display.composer_opacity,
                    ),
                    0.0,
                    1.0,
                ),
                menu_opacity: clamp(
                    number(display, "menuOpacity", defaults.display.menu_opacity),
                    0.0,
                    1.0,
                ),
                terminal_opacity: clamp(
                    number(
                        display,
                        "terminalOpacity",
                        defaults.display.terminal_opacity,
                    ),
                    0.0,
                    1.0,
                ),
                enabled_on_home: boolean(
                    display,
                    "enabledOnHome",
                    defaults.display.enabled_on_home,
                ),
                enabled_on_tasks: boolean(
                    display,
                    "enabledOnTasks",
                    defaults.display.enabled_on_tasks,
                ),
                video_muted: boolean(display, "videoMuted", defaults.display.video_muted),
                video_playback_rate: clamp(
                    number(
                        display,
                        "videoPlaybackRate",
                        defaults.display.video_playback_rate,
                    ),
                    0.25,
                    2.0,
                ),
            },
            slideshow: SlideshowSettings {
                enabled: boolean(slideshow, "enabled", defaults.slideshow.enabled),
                interval_seconds: clamp(
                    number(
                        slideshow,
                        "intervalSeconds",
                        defaults.slideshow.interval_seconds as f64,
                    ),
                    10.0,
                    86_400.0,
                )
                .round() as u64,
                order,
            },
            behavior: BehaviorSettings {
                close_to_tray: boolean(behavior, "closeToTray", defaults.behavior.close_to_tray),
                start_minimized: boolean(
                    behavior,
                    "startMinimized",
                    defaults.behavior.start_minimized,
                ),
                auto_start_with_windows: boolean(
                    behavior,
                    "autoStartWithWindows",
                    defaults.behavior.auto_start_with_windows,
                ),
                launch_multica_on_apply: boolean(
                    behavior,
                    "launchMulticaOnApply",
                    defaults.behavior.launch_multica_on_apply,
                ),
            },
        }
    }

    pub fn apply_patch(&mut self, patch: SettingsPatch) {
        if let Some(id) = patch.active_media_id {
            self.active_media_id = id.filter(|id| id.len() <= 120);
        }
        if let Some(ids) = patch.playlist_ids {
            self.playlist_ids.clear();
            for id in ids.into_iter().filter(|id| id.len() <= 120) {
                if !self.playlist_ids.contains(&id) {
                    self.playlist_ids.push(id);
                }
            }
        }
        if let Some(patch) = patch.display {
            if let Some(value) = patch.fit {
                self.display.fit = value;
            }
            macro_rules! set_clamped {
                ($field:ident, $minimum:expr, $maximum:expr) => {
                    if let Some(value) = patch.$field.filter(|value| value.is_finite()) {
                        self.display.$field = clamp(value, $minimum, $maximum);
                    }
                };
            }
            set_clamped!(position_x, 0.0, 100.0);
            set_clamped!(position_y, 0.0, 100.0);
            set_clamped!(opacity, 0.0, 1.0);
            set_clamped!(blur, 0.0, 40.0);
            set_clamped!(scale, 1.0, 1.3);
            set_clamped!(overlay_opacity, 0.0, 0.9);
            set_clamped!(block_fill_opacity, 0.0, 1.0);
            set_clamped!(home_intensity, 0.0, 1.0);
            set_clamped!(task_intensity, 0.0, 1.0);
            set_clamped!(sidebar_opacity, 0.0, 1.0);
            set_clamped!(surface_opacity, 0.0, 1.0);
            set_clamped!(card_opacity, 0.0, 1.0);
            set_clamped!(composer_opacity, 0.0, 1.0);
            set_clamped!(menu_opacity, 0.0, 1.0);
            set_clamped!(terminal_opacity, 0.0, 1.0);
            set_clamped!(video_playback_rate, 0.25, 2.0);
            if let Some(color) = patch.overlay_color.filter(|color| {
                color.len() == 7
                    && color.starts_with('#')
                    && color[1..]
                        .chars()
                        .all(|character| character.is_ascii_hexdigit())
            }) {
                self.display.overlay_color = color.to_ascii_lowercase();
            }
            macro_rules! set_boolean {
                ($field:ident) => {
                    if let Some(value) = patch.$field {
                        self.display.$field = value;
                    }
                };
            }
            set_boolean!(enabled_on_home);
            set_boolean!(enabled_on_tasks);
            set_boolean!(video_muted);
        }
        if let Some(patch) = patch.slideshow {
            if let Some(value) = patch.enabled {
                self.slideshow.enabled = value;
            }
            if let Some(value) = patch.interval_seconds.filter(|value| value.is_finite()) {
                self.slideshow.interval_seconds = clamp(value, 10.0, 86_400.0).round() as u64;
            }
            if let Some(value) = patch.order {
                self.slideshow.order = value;
            }
        }
        if let Some(patch) = patch.behavior {
            macro_rules! set_behavior {
                ($field:ident) => {
                    if let Some(value) = patch.$field {
                        self.behavior.$field = value;
                    }
                };
            }
            set_behavior!(close_to_tray);
            set_behavior!(start_minimized);
            set_behavior!(auto_start_with_windows);
            set_behavior!(launch_multica_on_apply);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_legacy_settings_and_allows_zero_opacity() {
        let settings = AppSettings::normalize(&json!({
            "activeMediaId": "背景-一号",
            "display": {
                "fit": "invalid",
                "opacity": 9,
                "blur": -4,
                "overlayColor": "red; background:url(x)",
                "sidebarOpacity": 0,
                "surfaceOpacity": 0,
                "cardOpacity": 0,
                "composerOpacity": 0,
                "menuOpacity": 0,
                "terminalOpacity": 0
            },
            "slideshow": { "intervalSeconds": 2, "order": "sideways" }
        }));
        assert_eq!(settings.active_media_id.as_deref(), Some("背景-一号"));
        assert_eq!(settings.display.opacity, 1.0);
        assert_eq!(settings.display.blur, 0.0);
        assert_eq!(settings.display.sidebar_opacity, 0.0);
        assert_eq!(settings.display.surface_opacity, 0.0);
        assert_eq!(settings.display.card_opacity, 0.0);
        assert_eq!(settings.display.composer_opacity, 0.0);
        assert_eq!(settings.display.menu_opacity, 0.0);
        assert_eq!(settings.display.terminal_opacity, 0.0);
        assert_eq!(settings.slideshow.interval_seconds, 10);
    }
}
