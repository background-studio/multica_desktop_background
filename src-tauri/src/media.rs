use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    models::{ImportResult, MediaItem, MediaKind, MediaOrigin, SkippedImport, SlideshowOrder},
    network::RemoteDownload,
    settings::write_json_transaction,
};

const MAX_IMAGE_BYTES: u64 = 50 * 1024 * 1024;
const MAX_VIDEO_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy)]
struct MediaType {
    kind: MediaKindStatic,
    mime_type: &'static str,
    maximum: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MediaKindStatic {
    Image,
    Video,
}

impl MediaKindStatic {
    fn owned(self) -> MediaKind {
        match self {
            Self::Image => MediaKind::Image,
            Self::Video => MediaKind::Video,
        }
    }
}

fn media_type(extension: &str) -> Option<MediaType> {
    match extension {
        ".png" => Some(MediaType {
            kind: MediaKindStatic::Image,
            mime_type: "image/png",
            maximum: MAX_IMAGE_BYTES,
        }),
        ".jpg" | ".jpeg" => Some(MediaType {
            kind: MediaKindStatic::Image,
            mime_type: "image/jpeg",
            maximum: MAX_IMAGE_BYTES,
        }),
        ".webp" => Some(MediaType {
            kind: MediaKindStatic::Image,
            mime_type: "image/webp",
            maximum: MAX_IMAGE_BYTES,
        }),
        ".gif" => Some(MediaType {
            kind: MediaKindStatic::Image,
            mime_type: "image/gif",
            maximum: MAX_IMAGE_BYTES,
        }),
        ".avif" => Some(MediaType {
            kind: MediaKindStatic::Image,
            mime_type: "image/avif",
            maximum: MAX_IMAGE_BYTES,
        }),
        ".mp4" => Some(MediaType {
            kind: MediaKindStatic::Video,
            mime_type: "video/mp4",
            maximum: MAX_VIDEO_BYTES,
        }),
        ".webm" => Some(MediaType {
            kind: MediaKindStatic::Video,
            mime_type: "video/webm",
            maximum: MAX_VIDEO_BYTES,
        }),
        ".ogv" => Some(MediaType {
            kind: MediaKindStatic::Video,
            mime_type: "video/ogg",
            maximum: MAX_VIDEO_BYTES,
        }),
        ".mov" => Some(MediaType {
            kind: MediaKindStatic::Video,
            mime_type: "video/quicktime",
            maximum: MAX_VIDEO_BYTES,
        }),
        _ => None,
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{}", value.to_ascii_lowercase()))
        .unwrap_or_default()
}

fn safe_display_name(name: &str) -> String {
    let value = name
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) || character.is_control()
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let trimmed = value.trim();
    let result = if trimmed.is_empty() {
        "未命名媒体".to_string()
    } else {
        trimmed.chars().take(180).collect()
    };
    result
}

fn sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_text(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

const FOLDER_SENTINEL: &str = "__folder__";
/// 同一次预览+应用不要连扫两遍；过了这点时间按目录可能已变处理。
const FOLDER_LISTING_TTL: Duration = Duration::from_secs(2);

struct CachedFolderListing {
    files: Arc<[PathBuf]>,
    scanned_at: Instant,
}

#[derive(Clone, Debug)]
pub struct ResolvedMedia {
    pub path: PathBuf,
    pub mime_type: String,
    pub byte_size: u64,
    pub kind: MediaKind,
}

fn validate_media(path: &Path, media_type: MediaType, extension: &str) -> Result<(), String> {
    let mut header = [0u8; 64];
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let count = file.read(&mut header).map_err(|error| error.to_string())?;
    let header = &header[..count];
    let matches = match extension {
        ".png" => header.starts_with(b"\x89PNG\r\n\x1a\n"),
        ".jpg" | ".jpeg" => header.starts_with(&[0xff, 0xd8, 0xff]),
        ".webp" => header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP",
        ".gif" => header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a"),
        ".avif" => {
            header.len() >= 12
                && &header[4..8] == b"ftyp"
                && header[8..]
                    .windows(4)
                    .any(|brand| matches!(brand, b"avif" | b"avis"))
        }
        ".webm" => header.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        ".ogv" => header.starts_with(b"OggS"),
        ".mp4" | ".mov" => header.len() >= 12 && &header[4..8] == b"ftyp",
        _ => false,
    };
    if !matches {
        return Err(match media_type.kind {
            MediaKindStatic::Image => "图片内容损坏或格式与扩展名不匹配。".to_string(),
            MediaKindStatic::Video => "视频内容损坏或格式与扩展名不匹配。".to_string(),
        });
    }
    if media_type.kind == MediaKindStatic::Image {
        let dimensions =
            imagesize::size(path).map_err(|_| "图片内容损坏或格式与扩展名不匹配。".to_string())?;
        let width = dimensions.width as u64;
        let height = dimensions.height as u64;
        if width < 1
            || height < 1
            || width > 16_384
            || height > 16_384
            || width.saturating_mul(height) > 50_000_000
        {
            return Err("图片尺寸超过 16384 像素或 5000 万总像素上限。".to_string());
        }
    }
    Ok(())
}

struct IngestOptions {
    name: Option<String>,
    origin: MediaOrigin,
    source_url: Option<String>,
    remove_source: bool,
    allow_duplicate: bool,
    extension: Option<String>,
}

pub struct MediaLibrary {
    pub media_directory: PathBuf,
    pub temporary_directory: PathBuf,
    pub catalog_path: PathBuf,
    items: Vec<MediaItem>,
    /// 文件夹源的顺序轮播游标；仅内存态，不写入 library.json。
    folder_cursors: HashMap<String, usize>,
    /// 文件夹源在随机模式下当前已经选中的文件，保证预览和 Multica 使用同一张图。
    folder_selections: HashMap<String, ResolvedMedia>,
    /// 文件夹文件列表。过期后换图、轮播、打开面板都会重新扫描。
    folder_listings: HashMap<String, CachedFolderListing>,
}

impl MediaLibrary {
    pub fn load(data_directory: &Path) -> Result<Self, String> {
        let media_directory = data_directory.join("media");
        let temporary_directory = data_directory.join("temporary");
        let catalog_path = data_directory.join("library.json");
        fs::create_dir_all(&media_directory).map_err(|error| error.to_string())?;
        fs::create_dir_all(&temporary_directory).map_err(|error| error.to_string())?;
        let mut library = Self {
            media_directory,
            temporary_directory,
            catalog_path,
            items: Vec::new(),
            folder_cursors: HashMap::new(),
            folder_selections: HashMap::new(),
            folder_listings: HashMap::new(),
        };
        match fs::read_to_string(&library.catalog_path) {
            Ok(content) => match serde_json::from_str::<Vec<MediaItem>>(&content) {
                Ok(items) => {
                    library.items = items
                        .into_iter()
                        .filter(|item| library.item_still_available(item))
                        .collect();
                }
                Err(_) => {
                    let invalid = library
                        .catalog_path
                        .with_extension(format!("json.invalid-{}", Utc::now().timestamp_millis()));
                    let _ = fs::rename(&library.catalog_path, invalid);
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                let invalid = library
                    .catalog_path
                    .with_extension(format!("json.invalid-{}", Utc::now().timestamp_millis()));
                let _ = fs::rename(&library.catalog_path, invalid);
            }
        }
        library.save_catalog()?;
        Ok(library)
    }

    fn item_still_available(&self, item: &MediaItem) -> bool {
        if item.origin == MediaOrigin::Folder {
            return item
                .source_url
                .as_deref()
                .map(Path::new)
                .is_some_and(|path| path.is_dir());
        }
        self.path_for(item)
            .and_then(|path| fs::metadata(path).map_err(|error| error.to_string()))
            .is_ok_and(|metadata| metadata.is_file())
    }

    pub fn items(&self) -> Vec<MediaItem> {
        self.items.clone()
    }

    pub fn get_by_id(&self, id: &str) -> Option<MediaItem> {
        self.items.iter().find(|item| item.id == id).cloned()
    }

    pub fn path_for(&self, item: &MediaItem) -> Result<PathBuf, String> {
        if item.origin == MediaOrigin::Folder {
            return Err("文件夹源没有受管媒体文件路径，请使用 resolve_playback。".to_string());
        }
        let path = Path::new(&item.file_name);
        let mut components = path.components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err("媒体目录校验失败。".to_string());
        }
        Ok(self.media_directory.join(path))
    }

    pub fn resolve_playback(
        &mut self,
        item: &MediaItem,
        order: SlideshowOrder,
        advance: bool,
    ) -> Result<ResolvedMedia, String> {
        if item.origin == MediaOrigin::Folder {
            if advance {
                self.advance_folder_cursor(item, order)?;
            }
            return self.resolve_folder_media(item, order);
        }
        let path = self.path_for(item)?;
        Ok(ResolvedMedia {
            path,
            mime_type: item.mime_type.clone(),
            byte_size: item.byte_size,
            kind: item.kind.clone(),
        })
    }

    fn folder_path_for(item: &MediaItem) -> Result<PathBuf, String> {
        let raw = item
            .source_url
            .as_deref()
            .ok_or_else(|| "文件夹源缺少路径。".to_string())?;
        let path = PathBuf::from(raw);
        if !path.is_dir() {
            return Err("文件夹源已不存在或不可访问。".to_string());
        }
        Ok(path)
    }

    fn folder_display_name(folder: &Path, count: u64) -> String {
        let folder_name = folder
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("未命名文件夹");
        safe_display_name(&format!("{folder_name}（{count}）"))
    }

    pub fn list_folder_media(folder: &Path) -> Result<Vec<PathBuf>, String> {
        let root = folder
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let mut pending = VecDeque::from([root]);
        let mut files = Vec::new();
        while let Some(directory) = pending.pop_front() {
            for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
                let entry = entry.map_err(|error| error.to_string())?;
                let file_type = entry.file_type().map_err(|error| error.to_string())?;
                if file_type.is_symlink() {
                    continue;
                }
                let path = entry.path();
                if file_type.is_dir() {
                    pending.push_back(path);
                } else if file_type.is_file() && media_type(&extension(&path)).is_some() {
                    files.push(path);
                }
                if files.len() + pending.len() > 10_000 {
                    return Err("文件夹内容过多，请选择更具体的目录。".to_string());
                }
            }
        }
        files.sort();
        Ok(files)
    }

    fn folder_files_cached(&mut self, item: &MediaItem) -> Result<Arc<[PathBuf]>, String> {
        if let Some(cached) = self.folder_listings.get(&item.id) {
            if cached.scanned_at.elapsed() < FOLDER_LISTING_TTL {
                return Ok(Arc::clone(&cached.files));
            }
        }
        self.rescan_folder_listing(item)
    }

    fn rescan_folder_listing(&mut self, item: &MediaItem) -> Result<Arc<[PathBuf]>, String> {
        let folder = Self::folder_path_for(item)?;
        let files: Arc<[PathBuf]> = Self::list_folder_media(&folder)?.into();
        let count = files.len() as u64;
        self.folder_listings.insert(
            item.id.clone(),
            CachedFolderListing {
                files: Arc::clone(&files),
                scanned_at: Instant::now(),
            },
        );
        let stored_count = self
            .items
            .iter()
            .find(|candidate| candidate.id == item.id)
            .and_then(|candidate| candidate.file_count);
        if stored_count != Some(count) {
            if let Some(stored) = self.items.iter_mut().find(|candidate| candidate.id == item.id) {
                stored.file_count = Some(count);
                stored.name = Self::folder_display_name(&folder, count);
            }
            self.save_catalog()?;
        }
        Ok(files)
    }

    pub fn refresh_folder_sources(&mut self) {
        let folder_items: Vec<MediaItem> = self
            .items
            .iter()
            .filter(|item| item.origin == MediaOrigin::Folder)
            .cloned()
            .collect();
        for item in folder_items {
            let _ = self.folder_files_cached(&item);
        }
    }

    #[cfg(test)]
    fn expire_folder_listings(&mut self) {
        let stale = Instant::now()
            .checked_sub(FOLDER_LISTING_TTL + Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        for listing in self.folder_listings.values_mut() {
            listing.scanned_at = stale;
        }
    }

    fn resolve_folder_media(
        &mut self,
        item: &MediaItem,
        order: SlideshowOrder,
    ) -> Result<ResolvedMedia, String> {
        let files = self.folder_files_cached(item)?;
        if files.is_empty() {
            return Err("文件夹中没有支持的图片或视频。".to_string());
        }
        let path = match order {
            SlideshowOrder::Random => {
                if let Some(selected_path) = self
                    .folder_selections
                    .get(&item.id)
                    .map(|selected| selected.path.clone())
                {
                    let still_listed = files.iter().any(|path| path == &selected_path);
                    if still_listed {
                        if let Ok(resolved) = Self::resolve_folder_file(selected_path) {
                            self.folder_selections
                                .insert(item.id.clone(), resolved.clone());
                            return Ok(resolved);
                        }
                    }
                    self.folder_selections.remove(&item.id);
                }
                let seed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default();
                files[(seed % files.len() as u128) as usize].clone()
            }
            SlideshowOrder::Sequential => {
                files[self.folder_cursors.get(&item.id).copied().unwrap_or(0) % files.len()]
                    .clone()
            }
        };
        let resolved = Self::resolve_folder_file(path)?;
        if matches!(order, SlideshowOrder::Random) {
            self.folder_selections
                .insert(item.id.clone(), resolved.clone());
        } else {
            self.folder_selections.remove(&item.id);
        }
        Ok(resolved)
    }

    fn resolve_folder_file(path: PathBuf) -> Result<ResolvedMedia, String> {
        let chosen_extension = extension(&path);
        let media_type =
            media_type(&chosen_extension).ok_or_else(|| "不支持此图片或视频格式。".to_string())?;
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if metadata.len() > media_type.maximum {
            return Err(format!(
                "媒体文件超过 {} MB 上限。",
                media_type.maximum / 1024 / 1024
            ));
        }
        Ok(ResolvedMedia {
            path,
            mime_type: media_type.mime_type.to_string(),
            byte_size: metadata.len(),
            kind: media_type.kind.owned(),
        })
    }

    pub fn advance_folder_cursor(
        &mut self,
        item: &MediaItem,
        order: SlideshowOrder,
    ) -> Result<(), String> {
        if item.origin != MediaOrigin::Folder {
            return Ok(());
        }
        let files = self.folder_files_cached(item)?;
        if files.is_empty() {
            return Err("文件夹中没有支持的图片或视频。".to_string());
        }
        if matches!(order, SlideshowOrder::Sequential) {
            let next = self
                .folder_cursors
                .get(&item.id)
                .copied()
                .unwrap_or(0)
                .saturating_add(1)
                % files.len();
            self.folder_cursors.insert(item.id.clone(), next);
            self.folder_selections.remove(&item.id);
        } else {
            let current = self
                .folder_selections
                .get(&item.id)
                .map(|selected| selected.path.clone());
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            let mut index = (seed % files.len() as u128) as usize;
            if files.len() > 1 && current.as_ref() == Some(&files[index]) {
                index = (index + 1) % files.len();
            }
            let selected = Self::resolve_folder_file(files[index].clone())?;
            self.folder_selections.insert(item.id.clone(), selected);
        }
        Ok(())
    }

    fn save_catalog(&self) -> Result<(), String> {
        write_json_transaction(&self.catalog_path, &self.items)
    }

    fn ingest(
        &mut self,
        source_path: &Path,
        options: IngestOptions,
    ) -> Result<(MediaItem, bool), String> {
        let source = source_path
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let chosen_extension = options
            .extension
            .unwrap_or_else(|| extension(options.name.as_deref().map(Path::new).unwrap_or(&source)))
            .to_ascii_lowercase();
        let media_type =
            media_type(&chosen_extension).ok_or_else(|| "不支持此图片或视频格式。".to_string())?;
        let metadata = fs::metadata(&source).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() < 1 {
            return Err("媒体文件为空或不可读取。".to_string());
        }
        if metadata.len() > media_type.maximum {
            return Err(format!(
                "媒体文件超过 {} MB 上限。",
                media_type.maximum / 1024 / 1024
            ));
        }
        validate_media(&source, media_type, &chosen_extension)?;
        let digest = sha256(&source)?;
        if !options.allow_duplicate {
            if let Some(item) = self.items.iter().find(|item| item.sha256 == digest) {
                if options.remove_source {
                    let _ = fs::remove_file(&source);
                }
                return Ok((item.clone(), true));
            }
        }

        let id = Uuid::new_v4().to_string();
        let stored_name = format!("{id}{chosen_extension}");
        let target = self.media_directory.join(&stored_name);
        let temporary = self.media_directory.join(format!(".{id}.incoming"));
        let copy_result = if options.remove_source {
            fs::rename(&source, &temporary).or_else(|_| {
                fs::copy(&source, &temporary)?;
                fs::remove_file(&source)
            })
        } else {
            fs::copy(&source, &temporary).map(|_| ())
        };
        if let Err(error) = copy_result {
            let _ = fs::remove_file(&temporary);
            return Err(error.to_string());
        }
        let result = (|| {
            let copied = fs::metadata(&temporary).map_err(|error| error.to_string())?;
            if copied.len() != metadata.len() {
                return Err("媒体复制校验失败。".to_string());
            }
            fs::rename(&temporary, &target).map_err(|error| error.to_string())?;
            let source_name = source
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("未命名媒体");
            let item = MediaItem {
                id,
                name: safe_display_name(options.name.as_deref().unwrap_or(source_name)),
                kind: media_type.kind.owned(),
                origin: options.origin,
                file_name: stored_name,
                mime_type: media_type.mime_type.to_string(),
                byte_size: metadata.len(),
                sha256: digest,
                source_url: options.source_url,
                file_count: None,
                created_at: Utc::now().to_rfc3339(),
                preview_url: None,
            };
            self.items.insert(0, item.clone());
            self.save_catalog()?;
            Ok(item)
        })();
        match result {
            Ok(item) => Ok((item, false)),
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                let _ = fs::remove_file(&target);
                Err(error)
            }
        }
    }

    pub fn import_files(&mut self, paths: &[PathBuf]) -> ImportResult {
        let mut result = ImportResult::default();
        for path in paths {
            match self.ingest(
                path,
                IngestOptions {
                    name: None,
                    origin: MediaOrigin::Local,
                    source_url: None,
                    remove_source: false,
                    allow_duplicate: false,
                    extension: None,
                },
            ) {
                Ok((_, true)) => result.skipped.push(SkippedImport {
                    path: path.to_string_lossy().into_owned(),
                    reason: "媒体已存在".to_string(),
                }),
                Ok((item, false)) => result.added.push(item),
                Err(error) => result.skipped.push(SkippedImport {
                    path: path.to_string_lossy().into_owned(),
                    reason: error,
                }),
            }
        }
        result
    }

    pub fn import_folder(&mut self, folder: &Path) -> ImportResult {
        let root = match folder.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                return ImportResult {
                    added: Vec::new(),
                    skipped: vec![SkippedImport {
                        path: folder.to_string_lossy().into_owned(),
                        reason: error.to_string(),
                    }],
                };
            }
        };
        if !root.is_dir() {
            return ImportResult {
                added: Vec::new(),
                skipped: vec![SkippedImport {
                    path: root.to_string_lossy().into_owned(),
                    reason: "请选择一个文件夹。".to_string(),
                }],
            };
        }
        let root_key = root.to_string_lossy().into_owned();
        if let Some(existing) = self.items.iter().find(|item| {
            item.origin == MediaOrigin::Folder && item.source_url.as_deref() == Some(root_key.as_str())
        }) {
            return ImportResult {
                added: Vec::new(),
                skipped: vec![SkippedImport {
                    path: root_key,
                    reason: format!("文件夹源已存在：{}", existing.name),
                }],
            };
        }
        let files = match Self::list_folder_media(&root) {
            Ok(files) => files,
            Err(error) => {
                return ImportResult {
                    added: Vec::new(),
                    skipped: vec![SkippedImport {
                        path: root_key,
                        reason: error,
                    }],
                };
            }
        };
        if files.is_empty() {
            return ImportResult {
                added: Vec::new(),
                skipped: vec![SkippedImport {
                    path: root_key,
                    reason: "文件夹中没有支持的图片或视频。".to_string(),
                }],
            };
        }
        let count = files.len() as u64;
        let item = MediaItem {
            id: Uuid::new_v4().to_string(),
            name: Self::folder_display_name(&root, count),
            kind: MediaKind::Image,
            origin: MediaOrigin::Folder,
            file_name: FOLDER_SENTINEL.to_string(),
            mime_type: "application/x-directory".to_string(),
            byte_size: 0,
            sha256: sha256_text(&root_key),
            source_url: Some(root_key.clone()),
            file_count: Some(count),
            created_at: Utc::now().to_rfc3339(),
            preview_url: None,
        };
        self.items.insert(0, item.clone());
        if let Err(error) = self.save_catalog() {
            self.items.retain(|candidate| candidate.id != item.id);
            return ImportResult {
                added: Vec::new(),
                skipped: vec![SkippedImport {
                    path: root_key,
                    reason: error,
                }],
            };
        }
        self.folder_listings.insert(
            item.id.clone(),
            CachedFolderListing {
                files: Arc::from(files),
                scanned_at: Instant::now(),
            },
        );
        ImportResult {
            added: vec![item],
            skipped: Vec::new(),
        }
    }

    pub fn import_download(
        &mut self,
        input_url: &str,
        dynamic: bool,
        download: RemoteDownload,
    ) -> ImportResult {
        let extension = extension(Path::new(&download.original_name));
        let hostname = url::Url::parse(input_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string());
        let result = self.ingest(
            &download.temporary_path,
            IngestOptions {
                name: Some(if dynamic {
                    format!("随机 API · {hostname}")
                } else {
                    download.original_name
                }),
                origin: if dynamic {
                    MediaOrigin::Api
                } else {
                    MediaOrigin::Remote
                },
                source_url: Some(if dynamic {
                    input_url.to_string()
                } else {
                    download.source_url
                }),
                remove_source: true,
                allow_duplicate: dynamic,
                extension: Some(extension),
            },
        );
        match result {
            Ok((_, true)) => ImportResult {
                added: Vec::new(),
                skipped: vec![SkippedImport {
                    path: input_url.to_string(),
                    reason: "媒体已存在".to_string(),
                }],
            },
            Ok((item, false)) => ImportResult {
                added: vec![item],
                skipped: Vec::new(),
            },
            Err(error) => {
                let _ = fs::remove_file(download.temporary_path);
                ImportResult {
                    added: Vec::new(),
                    skipped: vec![SkippedImport {
                        path: input_url.to_string(),
                        reason: error,
                    }],
                }
            }
        }
    }

    pub fn refresh_with_download(
        &mut self,
        id: &str,
        download: RemoteDownload,
    ) -> Result<MediaItem, String> {
        let index = self
            .items
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(|| "媒体项目不存在。".to_string())?;
        let item = self.items[index].clone();
        if item.origin != MediaOrigin::Api || item.source_url.is_none() {
            let _ = fs::remove_file(download.temporary_path);
            return Err("该媒体不是随机 API 来源。".to_string());
        }
        let extension = extension(Path::new(&download.original_name));
        let media_type =
            media_type(&extension).ok_or_else(|| "不支持此图片或视频格式。".to_string())?;
        validate_media(&download.temporary_path, media_type, &extension)?;
        let digest = sha256(&download.temporary_path)?;
        if digest == item.sha256 {
            let _ = fs::remove_file(download.temporary_path);
            return Ok(item);
        }
        let previous = self.path_for(&item)?;
        let stored_name = format!("{}-{}{}", item.id, &digest[..12], extension);
        let target = self.media_directory.join(&stored_name);
        fs::rename(&download.temporary_path, &target)
            .or_else(|_| {
                fs::copy(&download.temporary_path, &target)?;
                fs::remove_file(&download.temporary_path)
            })
            .map_err(|error| error.to_string())?;
        if previous != target {
            let _ = fs::remove_file(previous);
        }
        let updated = MediaItem {
            file_name: stored_name,
            mime_type: download.mime_type,
            kind: download.kind,
            byte_size: download.byte_size,
            sha256: digest,
            preview_url: None,
            ..item
        };
        self.items[index] = updated.clone();
        self.save_catalog()?;
        Ok(updated)
    }

    pub fn remove(&mut self, id: &str) -> Result<bool, String> {
        let Some(item) = self.get_by_id(id) else {
            return Ok(false);
        };
        self.items.retain(|candidate| candidate.id != id);
        self.folder_cursors.remove(id);
        self.folder_selections.remove(id);
        self.folder_listings.remove(id);
        self.save_catalog()?;
        if item.origin == MediaOrigin::Folder {
            // 文件夹源只删除目录引用，绝不触碰用户原目录里的文件。
            return Ok(true);
        }
        let path = self.path_for(&item)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn png_header() -> Vec<u8> {
        let mut bytes = vec![0u8; 24];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes[8..12].copy_from_slice(&13u32.to_be_bytes());
        bytes[12..16].copy_from_slice(b"IHDR");
        bytes[16..20].copy_from_slice(&2u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&2u32.to_be_bytes());
        bytes
    }

    #[test]
    fn imports_deduplicates_and_removes_media() {
        let root = std::env::temp_dir().join(format!("codex-media-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("中文背景.png");
        File::create(&source)
            .unwrap()
            .write_all(&png_header())
            .unwrap();
        let mut library = MediaLibrary::load(&root.join("data")).unwrap();
        let first = library.import_files(std::slice::from_ref(&source));
        let duplicate = library.import_files(std::slice::from_ref(&source));
        assert_eq!(first.added.len(), 1);
        assert_eq!(first.added[0].name, "中文背景.png");
        assert!(duplicate.added.is_empty());
        assert_eq!(duplicate.skipped[0].reason, "媒体已存在");
        assert!(library.remove(&first.added[0].id).unwrap());
        assert!(library.items().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn imports_folder_as_path_reference_without_copying() {
        let root = std::env::temp_dir().join(format!("codex-folder-{}", Uuid::new_v4()));
        let source_dir = root.join("wallpapers");
        fs::create_dir_all(&source_dir).unwrap();
        for index in 0..3 {
            let path = source_dir.join(format!("bg-{index}.png"));
            File::create(&path)
                .unwrap()
                .write_all(&png_header())
                .unwrap();
        }
        let mut library = MediaLibrary::load(&root.join("data")).unwrap();
        let imported = library.import_folder(&source_dir);
        assert_eq!(imported.added.len(), 1);
        assert_eq!(imported.added[0].origin, MediaOrigin::Folder);
        assert_eq!(imported.added[0].file_count, Some(3));
        assert!(imported.added[0].source_url.as_ref().unwrap().contains("wallpapers"));
        // 受管 media 目录不应出现用户原图副本。
        let managed = fs::read_dir(&library.media_directory).unwrap().count();
        assert_eq!(managed, 0);
        let resolved = library
            .resolve_playback(&imported.added[0], SlideshowOrder::Sequential, false)
            .unwrap();
        // Windows canonicalize 会加 \\?\ 前缀，断言要用同一规范化路径。
        let canonical_source = source_dir.canonicalize().unwrap();
        assert!(resolved.path.starts_with(&canonical_source));
        assert!(library.remove(&imported.added[0].id).unwrap());
        assert!(source_dir.join("bg-0.png").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn random_folder_refresh_keeps_preview_and_playback_on_same_file() {
        let root = std::env::temp_dir().join(format!("codex-folder-random-{}", Uuid::new_v4()));
        let source_dir = root.join("wallpapers");
        fs::create_dir_all(&source_dir).unwrap();
        for index in 0..3 {
            let path = source_dir.join(format!("bg-{index}.png"));
            File::create(&path)
                .unwrap()
                .write_all(&png_header())
                .unwrap();
        }
        let mut library = MediaLibrary::load(&root.join("data")).unwrap();
        let mut imported = library.import_folder(&source_dir);
        let item = imported.added.remove(0);

        let first = library
            .resolve_playback(&item, SlideshowOrder::Random, false)
            .unwrap();
        let same_selection = library
            .resolve_playback(&item, SlideshowOrder::Random, false)
            .unwrap();
        assert_eq!(first.path, same_selection.path);

        library
            .advance_folder_cursor(&item, SlideshowOrder::Random)
            .unwrap();
        let refreshed = library
            .resolve_playback(&item, SlideshowOrder::Random, false)
            .unwrap();
        assert_ne!(first.path, refreshed.path);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn folder_listing_is_reused_within_ttl() {
        let root = std::env::temp_dir().join(format!("codex-folder-cache-{}", Uuid::new_v4()));
        let source_dir = root.join("wallpapers");
        fs::create_dir_all(&source_dir).unwrap();
        for index in 0..2 {
            let path = source_dir.join(format!("bg-{index}.png"));
            File::create(&path)
                .unwrap()
                .write_all(&png_header())
                .unwrap();
        }
        let mut library = MediaLibrary::load(&root.join("data")).unwrap();
        let mut imported = library.import_folder(&source_dir);
        let item = imported.added.remove(0);
        assert_eq!(library.folder_listings[&item.id].files.len(), 2);

        File::create(source_dir.join("added-later.png"))
            .unwrap()
            .write_all(&png_header())
            .unwrap();
        library
            .advance_folder_cursor(&item, SlideshowOrder::Sequential)
            .unwrap();
        let selected = library
            .resolve_playback(&item, SlideshowOrder::Sequential, false)
            .unwrap();

        assert_eq!(library.folder_listings[&item.id].files.len(), 2);
        assert_eq!(library.get_by_id(&item.id).unwrap().file_count, Some(2));
        assert_ne!(
            selected.path.file_name().and_then(|name| name.to_str()),
            Some("added-later.png")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn folder_listing_picks_up_changes_after_ttl() {
        let root = std::env::temp_dir().join(format!("codex-folder-refresh-{}", Uuid::new_v4()));
        let source_dir = root.join("wallpapers");
        fs::create_dir_all(&source_dir).unwrap();
        for index in 0..2 {
            let path = source_dir.join(format!("bg-{index}.png"));
            File::create(&path)
                .unwrap()
                .write_all(&png_header())
                .unwrap();
        }
        let mut library = MediaLibrary::load(&root.join("data")).unwrap();
        let mut imported = library.import_folder(&source_dir);
        let item = imported.added.remove(0);

        File::create(source_dir.join("added-later.png"))
            .unwrap()
            .write_all(&png_header())
            .unwrap();
        library.expire_folder_listings();
        library.refresh_folder_sources();

        let updated = library.get_by_id(&item.id).unwrap();
        assert_eq!(updated.file_count, Some(3));
        assert!(updated.name.contains("（3）"));
        assert_eq!(library.folder_listings[&item.id].files.len(), 3);
        assert!(library.folder_listings[&item.id]
            .files
            .iter()
            .any(|path| path.file_name().and_then(|name| name.to_str()) == Some("added-later.png")));

        fs::remove_file(source_dir.join("bg-0.png")).unwrap();
        library.expire_folder_listings();
        library.refresh_folder_sources();
        let updated = library.get_by_id(&item.id).unwrap();
        assert_eq!(updated.file_count, Some(2));
        assert!(updated.name.contains("（2）"));
        assert_eq!(library.folder_listings[&item.id].files.len(), 2);

        fs::remove_file(source_dir.join("bg-1.png")).unwrap();
        fs::remove_file(source_dir.join("added-later.png")).unwrap();
        library.expire_folder_listings();
        let error = library
            .resolve_playback(&item, SlideshowOrder::Sequential, false)
            .unwrap_err();
        assert!(error.contains("没有支持的图片或视频"));
        assert_eq!(library.get_by_id(&item.id).unwrap().file_count, Some(0));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn random_folder_skips_deleted_selection() {
        let root = std::env::temp_dir().join(format!("codex-folder-deleted-{}", Uuid::new_v4()));
        let source_dir = root.join("wallpapers");
        fs::create_dir_all(&source_dir).unwrap();
        for index in 0..2 {
            let path = source_dir.join(format!("bg-{index}.png"));
            File::create(&path)
                .unwrap()
                .write_all(&png_header())
                .unwrap();
        }
        let mut library = MediaLibrary::load(&root.join("data")).unwrap();
        let mut imported = library.import_folder(&source_dir);
        let item = imported.added.remove(0);
        let first = library
            .resolve_playback(&item, SlideshowOrder::Random, false)
            .unwrap();
        fs::remove_file(&first.path).unwrap();
        library.expire_folder_listings();
        let next = library
            .resolve_playback(&item, SlideshowOrder::Random, false)
            .unwrap();
        assert_ne!(next.path, first.path);
        assert!(next.path.is_file());
        assert_eq!(library.get_by_id(&item.id).unwrap().file_count, Some(1));
        let _ = fs::remove_dir_all(root);
    }
}
