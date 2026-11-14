//! Loads Marketplace listing thumbnails for display. Images stay in memory only.

use bedrock_foundry::marketplace::{bounded_response, client, validate_asset_url};
use gpui::{Image, ImageFormat};
use std::{
    collections::HashMap,
    sync::{Arc, mpsc},
};

const MAX_IMAGE: usize = 2 * 1024 * 1024;
const MAX_CACHED: usize = 240;
const PARALLEL: usize = 4;

enum Thumb {
    Loading,
    Ready(Arc<Image>),
    Failed,
}

pub struct Thumbs {
    tx: Option<mpsc::Sender<String>>,
    rx: mpsc::Receiver<(String, Option<Arc<Image>>)>,
    cache: HashMap<String, Thumb>,
}

impl Thumbs {
    pub fn start() -> Self {
        let (tx, requests) = mpsc::channel::<String>();
        let (results, rx) = mpsc::channel();
        let started = std::thread::Builder::new()
            .name("thumbnails".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                else {
                    return;
                };
                let Ok(http) = client() else { return };
                let limit = Arc::new(tokio::sync::Semaphore::new(PARALLEL));
                while let Ok(url) = requests.recv() {
                    let http = http.clone();
                    let results = results.clone();
                    let limit = limit.clone();
                    runtime.spawn(async move {
                        let _permit = limit.acquire_owned().await;
                        let image = fetch(&http, &url).await;
                        let _ = results.send((url, image));
                    });
                }
            })
            .is_ok();
        Self {
            tx: started.then_some(tx),
            rx,
            cache: HashMap::new(),
        }
    }

    /// Returns a loaded image, and requests it the first time it is seen.
    pub fn get(&mut self, url: &str) -> Option<Arc<Image>> {
        match self.cache.get(url) {
            Some(Thumb::Ready(image)) => Some(image.clone()),
            Some(_) => None,
            None => {
                if self.cache.len() >= MAX_CACHED {
                    self.cache
                        .retain(|_, thumb| matches!(thumb, Thumb::Loading));
                    if self.cache.len() >= MAX_CACHED {
                        return None;
                    }
                }
                let sent = self
                    .tx
                    .as_ref()
                    .is_some_and(|tx| tx.send(url.to_owned()).is_ok());
                self.cache.insert(
                    url.to_owned(),
                    if sent { Thumb::Loading } else { Thumb::Failed },
                );
                None
            }
        }
    }

    /// Applies finished downloads. Returns true when the view should redraw.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((url, image)) = self.rx.try_recv() {
            changed = true;
            self.cache.insert(
                url,
                match image {
                    Some(image) => Thumb::Ready(image),
                    None => Thumb::Failed,
                },
            );
        }
        changed
    }
}

async fn fetch(http: &reqwest::Client, url: &str) -> Option<Arc<Image>> {
    let url = validate_asset_url(url).ok()?;
    let response = http.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = bounded_response(response, MAX_IMAGE).await.ok()?;
    let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ImageFormat::Png
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        ImageFormat::Jpeg
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else {
        return None;
    };
    Some(Arc::new(Image::from_bytes(format, bytes)))
}
