use crate::model::PickerSnapshot;
use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Files used to transfer picker state and release its launch barrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerLaunchFiles {
    pub snapshot_path: PathBuf,
    pub ready_path: PathBuf,
    pub(crate) marker_temp_path: PathBuf,
}

impl PickerLaunchFiles {
    /// Allocates unique absent paths and writes the snapshot.
    pub fn create(snapshot: &PickerSnapshot) -> Result<Self> {
        let stem = std::env::temp_dir().join(unique_stem());
        let files = Self::for_paths(&stem.with_extension("json"), &stem.with_extension("ready"));
        files.cleanup()?;
        let json = serde_json::to_vec(snapshot).context("failed to serialize picker snapshot")?;
        write_private(&files.snapshot_path, &json)?;
        Ok(files)
    }

    /// Names the launch files of an existing launch from its snapshot and barrier paths.
    pub fn for_paths(snapshot_path: &Path, ready_path: &Path) -> Self {
        Self {
            snapshot_path: snapshot_path.to_path_buf(),
            ready_path: ready_path.to_path_buf(),
            marker_temp_path: ready_path.with_extension("ready.tmp"),
        }
    }

    /// Atomically releases the picker after layout construction completes.
    pub fn signal_ready(&self) -> Result<()> {
        write_private(&self.marker_temp_path, b"ready")?;
        fs::rename(&self.marker_temp_path, &self.ready_path).with_context(|| {
            format!(
                "failed to signal picker readiness at {}",
                self.ready_path.display()
            )
        })
    }

    /// Removes all launch files, ignoring already-removed files.
    pub fn cleanup(&self) -> Result<()> {
        let mut first = None;
        for path in [
            &self.snapshot_path,
            &self.ready_path,
            &self.marker_temp_path,
        ] {
            if let Err(error) = remove_file(path) {
                if first.is_none() {
                    first = Some(error);
                }
            }
        }
        first.map_or(Ok(()), Err)
    }
}

/// Waits a bounded duration for the launch barrier.
pub fn wait_for_ready(path: &Path, timeout: Duration) -> Result<()> {
    let started = Instant::now();
    while !path.exists() {
        if started.elapsed() >= timeout {
            bail!(
                "timed out waiting for Herdr layout launch barrier at {}",
                path.display()
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// Reads the launch snapshot and removes it immediately.
///
/// The snapshot holds the captured pane text, so it is deleted as soon as it reaches memory
/// instead of at picker shutdown. That keeps the on-disk copy alive for microseconds rather
/// than for the whole interactive session.
pub fn take_snapshot_file(path: &Path) -> Result<PickerSnapshot> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let snapshot = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    remove_file(path)?;
    Ok(snapshot)
}

/// Creates an owner-only file, refusing to write through a pre-existing path or symlink.
///
/// `create_new` maps to `O_CREAT | O_EXCL`, so a hostile symlink planted at the predictable
/// temp path makes the open fail instead of redirecting the write.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn unique_stem() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let counter = FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("herdr-yank-{millis}-{}-{counter}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PickerAction, PickerReturnContext, PickerSettings, SourcePaneSnapshot};
    use std::os::unix::fs::PermissionsExt;

    fn snapshot() -> PickerSnapshot {
        PickerSnapshot {
            source: SourcePaneSnapshot {
                rows: vec!["secret token".into()],
                content_rows: 24,
            },
            session: PickerReturnContext {
                return_tab_id: "t1".into(),
                zoom_picker: false,
            },
            action: PickerAction::Copy,
            custom_patterns: Vec::new(),
            settings: PickerSettings::default(),
        }
    }

    #[test]
    fn private_write_is_owner_only_and_refuses_existing_paths() {
        let path = std::env::temp_dir().join(format!("yank-private-{}", std::process::id()));
        let _ = fs::remove_file(&path);

        write_private(&path, b"secret").unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(write_private(&path, b"again").is_err());
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn taking_a_snapshot_removes_it_from_disk() {
        let path = std::env::temp_dir().join(format!("yank-take-{}.json", std::process::id()));
        let _ = fs::remove_file(&path);
        write_private(&path, &serde_json::to_vec(&snapshot()).unwrap()).unwrap();

        let loaded = take_snapshot_file(&path).unwrap();

        assert_eq!(loaded.source.rows, vec!["secret token".to_string()]);
        assert!(!path.exists());
    }

    #[test]
    fn created_files_share_one_stem_and_clean_up() {
        let files = PickerLaunchFiles::create(&snapshot()).unwrap();

        assert_eq!(
            files,
            PickerLaunchFiles::for_paths(&files.snapshot_path, &files.ready_path)
        );
        assert!(files
            .marker_temp_path
            .to_string_lossy()
            .ends_with(".ready.tmp"));
        files.signal_ready().unwrap();
        assert!(files.ready_path.exists());
        files.cleanup().unwrap();
        assert!(!files.snapshot_path.exists() && !files.ready_path.exists());
    }

    #[test]
    fn barrier_releases_and_cleanup_is_idempotent() {
        let path = std::env::temp_dir().join(format!("yank-barrier-test-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        let writer = path.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            fs::write(writer, b"x").unwrap();
        });
        wait_for_ready(&path, Duration::from_secs(1)).unwrap();
        remove_file(&path).unwrap();
        remove_file(&path).unwrap();
    }

    #[test]
    fn barrier_times_out() {
        let path = std::env::temp_dir().join(format!("yank-missing-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        assert!(wait_for_ready(&path, Duration::from_millis(20))
            .unwrap_err()
            .to_string()
            .contains("timed out"));
    }
}
