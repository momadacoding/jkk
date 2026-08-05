use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};
use crate::error::{GitError, Result};

static WATCHER_HANDLE: Mutex<Option<AppHandle>> = Mutex::new(None);
static WATCHER_STOP: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub fn watch_repo(handle: AppHandle, path: String) -> Result<()> {
    {
        let mut h = WATCHER_HANDLE.lock().map_err(|_| GitError::new("mutex", "lock failed"))?;
        if h.is_some() {
            return Ok(());
        }
        *h = Some(handle.clone());
    }
    WATCHER_STOP.store(false, Ordering::SeqCst);

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<Event, notify::Error>>();
        let mut watcher: RecommendedWatcher = match RecommendedWatcher::new(
            move |res| { let _ = tx.send(res); },
            Config::default().with_poll_interval(Duration::from_millis(500)),
        ) {
            Ok(w) => w,
            Err(e) => {
                let _ = handle.emit("watch-error", e.to_string());
                return;
            }
        };

        if let Err(e) = watcher.watch(std::path::Path::new(&path), RecursiveMode::Recursive) {
            let _ = handle.emit("watch-error", e.to_string());
            return;
        }

        let mut last_emit = std::time::Instant::now();
        loop {
            if WATCHER_STOP.load(Ordering::SeqCst) {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Ok(event)) => {
                    let relevant = event.paths.iter().any(|p| {
                        let s = p.to_string_lossy();
                        s.contains("/.git/") || s.ends_with("/.git")
                    });
                    if !relevant {
                        continue;
                    }
                    if last_emit.elapsed() > Duration::from_millis(300) {
                        last_emit = std::time::Instant::now();
                        let _ = handle.emit("repo-changed", &path);
                    }
                }
                Ok(Err(e)) => {
                    let _ = handle.emit("watch-error", e.to_string());
                }
                Err(_) => {}
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub fn unwatch_repo() -> Result<()> {
    WATCHER_STOP.store(true, Ordering::SeqCst);
    let mut h = WATCHER_HANDLE.lock().map_err(|_| GitError::new("mutex", "lock failed"))?;
    *h = None;
    Ok(())
}
