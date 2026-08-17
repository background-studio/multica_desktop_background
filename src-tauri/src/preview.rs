use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use uuid::Uuid;

use crate::{
    media::{MediaLibrary, ResolvedMedia},
    models::SlideshowOrder,
};

#[derive(Clone)]
struct ServedMedia {
    path: PathBuf,
    mime_type: String,
    byte_size: u64,
    revision: String,
}

fn preview_revision(media: &ResolvedMedia) -> String {
    // URL 需要随文件夹当前选中文件变化，否则 WebView 会继续复用旧缩略图。
    let mut hasher = Sha256::new();
    hasher.update(media.path.to_string_lossy().as_bytes());
    hasher.update(media.byte_size.to_le_bytes());
    format!("{:x}", hasher.finalize())
}

fn header(name: &str, value: impl AsRef<str>) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_ref().as_bytes())
        .expect("static HTTP header is valid")
}

fn common_headers(mime_type: &str) -> Vec<Header> {
    vec![
        header("Content-Type", mime_type),
        header("Accept-Ranges", "bytes"),
        header("Cache-Control", "private, max-age=3600"),
        header("Cross-Origin-Resource-Policy", "cross-origin"),
        header("Access-Control-Allow-Origin", "*"),
        header("X-Content-Type-Options", "nosniff"),
    ]
}

fn plain(request: Request, status: u16) {
    let response = Response::from_string("")
        .with_status_code(StatusCode(status))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("Access-Control-Allow-Origin", "*"));
    let _ = request.respond(response);
}

fn parse_range(request: &Request, size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(value) = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Range"))
        .map(|header| header.value.as_str())
    else {
        return Ok(None);
    };
    let Some(range) = value.strip_prefix("bytes=") else {
        return Err(());
    };
    if range.contains(',') {
        return Err(());
    }
    let Some((start, end)) = range.split_once('-') else {
        return Err(());
    };
    let start = if start.is_empty() {
        0
    } else {
        start.parse::<u64>().map_err(|_| ())?
    };
    let requested_end = if end.is_empty() {
        size.saturating_sub(1)
    } else {
        end.parse::<u64>().map_err(|_| ())?
    };
    if size == 0 || start >= size || requested_end < start {
        return Err(());
    }
    Ok(Some((start, requested_end.min(size - 1))))
}

fn serve(request: Request, token: &str, media: &Arc<RwLock<HashMap<String, ServedMedia>>>) {
    if !matches!(request.method(), Method::Get | Method::Head) {
        plain(request, 405);
        return;
    }
    let path = request.url().split('?').next().unwrap_or(request.url());
    let prefix = format!("/{token}/media/");
    let Some(id) = path.strip_prefix(&prefix) else {
        plain(request, 404);
        return;
    };
    if id.len() != 36 || id.contains('/') {
        plain(request, 404);
        return;
    }
    let Some(item) = media.read().ok().and_then(|items| items.get(id).cloned()) else {
        plain(request, 404);
        return;
    };
    let Ok(mut file) = File::open(&item.path) else {
        plain(request, 404);
        return;
    };
    let size = file
        .metadata()
        .map(|metadata| metadata.len())
        .unwrap_or(item.byte_size);
    let range = match parse_range(&request, size) {
        Ok(range) => range,
        Err(()) => {
            let response = Response::empty(StatusCode(416))
                .with_header(header("Content-Range", format!("bytes */{size}")));
            let _ = request.respond(response);
            return;
        }
    };
    let is_head = request.method() == &Method::Head;
    let mut headers = common_headers(&item.mime_type);
    match range {
        Some((start, end)) => {
            let length = end - start + 1;
            headers.push(header(
                "Content-Range",
                format!("bytes {start}-{end}/{size}"),
            ));
            if is_head {
                headers.push(header("Content-Length", length.to_string()));
                let mut response = Response::empty(StatusCode(206));
                for header in headers {
                    response.add_header(header);
                }
                let _ = request.respond(response);
                return;
            }
            if file.seek(SeekFrom::Start(start)).is_err() {
                plain(request, 404);
                return;
            }
            let response = Response::new(
                StatusCode(206),
                headers,
                file.take(length),
                Some(length as usize),
                None,
            );
            let _ = request.respond(response);
        }
        None => {
            if is_head {
                headers.push(header("Content-Length", size.to_string()));
                let mut response = Response::empty(StatusCode(200));
                for header in headers {
                    response.add_header(header);
                }
                let _ = request.respond(response);
                return;
            }
            let response = Response::new(StatusCode(200), headers, file, Some(size as usize), None);
            let _ = request.respond(response);
        }
    }
}

pub struct MediaServer {
    token: String,
    origin: String,
    media: Arc<RwLock<HashMap<String, ServedMedia>>>,
    server: Arc<Server>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MediaServer {
    pub fn start(library: &mut MediaLibrary, order: SlideshowOrder) -> Result<Self, String> {
        let server = Arc::new(Server::http("127.0.0.1:0").map_err(|error| error.to_string())?);
        let port = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| "媒体服务端口分配失败。".to_string())?
            .port();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let media = Arc::new(RwLock::new(HashMap::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_server = Arc::clone(&server);
        let thread_media = Arc::clone(&media);
        let thread_stopping = Arc::clone(&stopping);
        let thread_token = token.clone();
        let thread = thread::Builder::new()
            .name("codex-media-preview".to_string())
            .spawn(move || {
                while !thread_stopping.load(Ordering::Relaxed) {
                    match thread_server.recv_timeout(Duration::from_millis(500)) {
                        Ok(Some(request)) => serve(request, &thread_token, &thread_media),
                        Ok(None) => {}
                        Err(_) if thread_stopping.load(Ordering::Relaxed) => break,
                        Err(_) => {}
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        let result = Self {
            token,
            origin: format!("http://127.0.0.1:{port}"),
            media,
            server,
            stopping,
            thread: Some(thread),
        };
        result.sync(library, order);
        Ok(result)
    }

    pub fn sync(&self, library: &mut MediaLibrary, order: SlideshowOrder) {
        let items = library
            .items()
            .into_iter()
            .filter_map(|item| {
                let resolved = if item.origin == crate::models::MediaOrigin::Folder {
                    library.resolve_playback(&item, order, false).ok()?
                } else {
                    let path = library.path_for(&item).ok()?;
                    crate::media::ResolvedMedia {
                        path,
                        mime_type: item.mime_type.clone(),
                        byte_size: item.byte_size,
                        kind: item.kind.clone(),
                    }
                };
                let revision = preview_revision(&resolved);
                Some((
                    item.id,
                    ServedMedia {
                        path: resolved.path,
                        mime_type: resolved.mime_type,
                        byte_size: resolved.byte_size,
                        revision,
                    },
                ))
            })
            .collect();
        if let Ok(mut media) = self.media.write() {
            *media = items;
        }
    }

    pub fn url_for(&self, id: &str) -> String {
        let revision = self
            .media
            .read()
            .ok()
            .and_then(|items| items.get(id).map(|item| item.revision.clone()))
            .unwrap_or_else(|| "missing".to_string());
        format!("{}/{}/media/{}?v={revision}", self.origin, self.token, id)
    }
}

impl Drop for MediaServer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        self.server.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
