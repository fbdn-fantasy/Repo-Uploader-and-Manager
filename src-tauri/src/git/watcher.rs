use super::*;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as NotifyWatcher};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, RecvTimeoutError},
    Arc,
};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Control {
    stopped: AtomicBool,
    #[cfg(any(windows, test))]
    process: std::sync::Mutex<Option<std::process::Child>>,
}

pub struct Watcher {
    _native: RecommendedWatcher,
    wake: mpsc::SyncSender<()>,
    control: Arc<Control>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.control.stopped.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
        #[cfg(any(windows, test))]
        if let Ok(mut guard) = self.control.process.lock() {
            if let Some(child) = guard.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

fn relevant(event: &Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    event.paths.is_empty()
        || event.paths.iter().any(|p| {
            let mut pieces = p.components();
            if !pieces.any(|v| v.as_os_str() == OsStr::new(".git")) {
                return true;
            }
            let mut git = pieces.map(|part| part.as_os_str());
            matches!(
                git.next().and_then(|v| v.to_str()),
                Some("HEAD" | "index" | "refs" | "packed-refs" | "config" | "MERGE_HEAD")
            ) && !p.extension().is_some_and(|s| s == "lock")
        })
}

#[cfg(any(windows, test))]
fn read_git_state(dir: &Path, control: &Control) -> Result<Vec<u8>> {
    use std::io::Read;
    let error =
        "Git could not check for missed filesystem events. Use Refresh or reopen the repository.";
    let mut guard = control.process.lock().map_err(|_| error)?;
    if control.stopped.load(Ordering::Acquire) {
        return Err(error.into());
    }
    let mut child = command()
        .current_dir(dir)
        .args([
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=all",
            "-z",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| error)?;
    let mut stdout = child.stdout.take().ok_or(error)?;
    *guard = Some(child);
    drop(guard);

    // Read without holding the lock. Deselecting a slow checkout can cancel
    // even an in-flight scan, closing its pipe and waking this worker.
    let mut state = Vec::new();
    let read = stdout.read_to_end(&mut state);
    let mut child = control
        .process
        .lock()
        .map_err(|_| error)?
        .take()
        .ok_or(error)?;
    let status = child.wait().map_err(|_| error)?;
    if read.is_err() || !status.success() {
        return Err(error.into());
    }
    Ok(state)
}

#[cfg(any(windows, test))]
fn reconcile(
    dir: &Path,
    control: &Control,
    previous: &mut Option<Result<Vec<u8>>>,
) -> Option<Result<()>> {
    let next = read_git_state(dir, control);
    if previous.as_ref() == Some(&next) {
        return None;
    }
    let refresh = next.as_ref().map(|_| ()).map_err(Clone::clone);
    *previous = Some(next);
    Some(refresh)
}

pub fn watch(dir: &Path, callback: impl Fn(Result<()>) + Send + 'static) -> Result<Watcher> {
    let dir = root(dir)?;
    let (wake, receive) = mpsc::sync_channel(1);
    let signal = wake.clone();
    let fault = Arc::new(AtomicBool::new(false));
    let native_fault = Arc::clone(&fault);
    let control = Arc::new(Control::default());
    let worker = Arc::clone(&control);
    let mut native = notify::recommended_watcher(move |result: notify::Result<Event>| {
        match result {
            Ok(event) if !relevant(&event) => return,
            Err(_) => native_fault.store(true, Ordering::Release),
            _ => (),
        }
        let _ = signal.try_send(());
    })
    .map_err(|_| "Filesystem monitoring is unavailable. Use Refresh to update Git status.")?;
    native.watch(&dir, RecursiveMode::Recursive)
        .map_err(|_| "Cannot watch the repository folder. Check file permissions and watcher limits, or use Refresh.")?;

    std::thread::Builder::new().name("repository-changes".into()).spawn(move || {
        #[cfg(windows)]
        let mut previous = None;
        loop {
            #[cfg(windows)]
            match receive.recv_timeout(Duration::from_secs(12)) {
                Ok(()) => {
                    // The UI may now observe an intermediate state independent
                    // of the last poll. Re-establish the baseline on the next poll.
                    previous = None;
                }
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    // Some Windows filesystems miss recursive notifications.
                    // Even the first sample requests a refresh to cover changes
                    // between the initial UI snapshot and watcher installation.
                    if let Some(refresh) = reconcile(&dir,&worker,&mut previous) {
                        if !worker.stopped.load(Ordering::Acquire) {callback(refresh);}
                    }
                    continue;
                }
            }
            #[cfg(not(windows))]
            if receive.recv().is_err() {return;}
            if worker.stopped.load(Ordering::Acquire) {return;}

            // Refresh after a quiet interval, or after one second of continuous changes.
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                let wait = deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(180));
                if wait.is_zero() {break;}
                match receive.recv_timeout(wait) {
                    Ok(()) if worker.stopped.load(Ordering::Acquire) => return,
                    Ok(()) => (),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            if worker.stopped.load(Ordering::Acquire) {return;}
            callback(if fault.swap(false, Ordering::AcqRel) {
                Err("Filesystem notifications were interrupted. Use Refresh or reopen the repository.".into())
            } else {Ok(())});
        }
    }).map_err(|_| "Could not start the filesystem monitor.")?;
    Ok(Watcher {
        _native: native,
        wake,
        control,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_refreshes_initial_state_and_recovers_missed_nested_events() {
        let repo = super::super::tests::working_tree();
        let dir = repo.path();
        let nested = dir.join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("early"), "before first poll").unwrap();
        let control = Control::default();
        let mut last = None;
        assert_eq!(reconcile(dir, &control, &mut last), Some(Ok(())));
        assert_eq!(reconcile(dir, &control, &mut last), None);
        std::fs::rename(nested.join("early"), nested.join("late")).unwrap();
        assert_eq!(reconcile(dir, &control, &mut last), Some(Ok(())));
        std::fs::remove_file(nested.join("late")).unwrap();
        assert_eq!(reconcile(dir, &control, &mut last), Some(Ok(())));
        assert_eq!(reconcile(dir, &control, &mut last), None);
    }
}
