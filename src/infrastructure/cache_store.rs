//! SFMTキャッシュのファイル配置と読み書きを担当する。
//!
//! 乱数列や検索条件の意味はドメイン層が管理し、このモジュールではファイル形式の
//! 検証、一時ファイル、保存先の解決だけを行う。

use crate::errors::{AppError, AppResult, CacheError, CacheIoOperation};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_MAGIC: &[u8; 8] = b"HBSFMT\0\0";

static CACHE_FILE_LEASES: OnceLock<
    Mutex<std::collections::HashMap<PathBuf, Weak<CacheFileLease>>>,
> = OnceLock::new();
static SESSION_CACHE_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
static CACHE_PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Keeps a temporary cache file alive while any operation still uses it.
pub(crate) struct CacheFileLease {
    pub(crate) path: PathBuf,
}

impl Drop for CacheFileLease {
    fn drop(&mut self) {
        schedule_cache_file_removal(self.path.clone());
    }
}

/// SFMT出力キャッシュの保存先と形式検証を担当するI/Oアダプター。
///
/// SFMT列の生成やTimeline計算は行わず、ファイルの場所・一時ファイル・ヘッダーだけを
/// 管理する。キャッシュが削除されても、呼び出し側が再生成できる前提を維持する。
pub(crate) struct CacheStore {
    directory: PathBuf,
}

impl Default for CacheStore {
    fn default() -> Self {
        Self {
            directory: session_cache_directory().clone(),
        }
    }
}

fn session_cache_directory() -> &'static PathBuf {
    SESSION_CACHE_DIRECTORY.get_or_init(|| {
        let root = std::env::temp_dir().join("earphone-lost-search-cache");
        let _ = fs::create_dir_all(&root);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let directory = root.join(format!("process-{}-{stamp}", std::process::id()));
        schedule_cache_root_cleanup(root, std::process::id(), directory.clone());
        directory
    })
}

fn schedule_cache_root_cleanup(root: PathBuf, current_pid: u32, current_directory: PathBuf) {
    let _ = std::thread::Builder::new()
        .name("sfmt-cache-cleanup".into())
        .spawn(move || {
            let _leases = CACHE_FILE_LEASES
                .get_or_init(|| Mutex::new(Default::default()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            remove_unleased_cache_files(&root, &std::collections::HashSet::new());
            remove_stale_process_directories(&root, current_pid, &current_directory);
        });
}

fn schedule_cache_file_removal(path: PathBuf) {
    let _ = std::thread::Builder::new()
        .name("sfmt-cache-cleanup".into())
        .spawn(move || {
            let mut leases = CACHE_FILE_LEASES
                .get_or_init(|| Mutex::new(Default::default()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if leases.get(&path).and_then(Weak::upgrade).is_none() {
                let _ = fs::remove_file(&path);
                leases.remove(&path);
            }
        });
}

fn clear_unused_cache_files(directory: &Path) {
    let mut leases = CACHE_FILE_LEASES
        .get_or_init(|| Mutex::new(Default::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    leases.retain(|_, lease| lease.strong_count() > 0);
    let active_paths = leases
        .iter()
        .filter(|(_, lease)| lease.strong_count() > 0)
        .map(|(path, _)| path.clone())
        .collect::<std::collections::HashSet<_>>();
    remove_unleased_cache_files(directory, &active_paths);

    if let Some(root) = directory.parent() {
        remove_unleased_cache_files(root, &active_paths);
        remove_stale_process_directories(root, std::process::id(), directory);
    }
}

fn remove_stale_process_directories(root: &Path, current_pid: u32, current_directory: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(pid) = name
            .strip_prefix("process-")
            .and_then(|suffix| suffix.split('-').next())
            .and_then(|pid| pid.parse().ok())
        else {
            continue;
        };
        let is_previous_session_of_current_process =
            pid == current_pid && path != current_directory;
        if is_previous_session_of_current_process
            || (pid != current_pid && !process_is_running(pid))
        {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut exit_code = 0;
        let query_succeeded = GetExitCodeProcess(process, &mut exit_code) != 0;
        CloseHandle(process);
        query_succeeded && exit_code == 259 // STILL_ACTIVE
    }
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }

    let result = unsafe { kill(pid as i32, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(1) // EPERM
}

#[cfg(not(any(unix, windows)))]
fn process_is_running(_pid: u32) -> bool {
    true
}

impl CacheStore {
    pub(crate) fn ensure_directory(&self) -> AppResult<()> {
        fs::create_dir_all(&self.directory).map_err(|source| {
            AppError::Cache(CacheError::Io {
                operation: CacheIoOperation::CreateDirectory,
                source,
            })
        })
    }

    pub(crate) fn path(&self, seed: u32, last_position: i64) -> PathBuf {
        self.directory
            .join(format!("sfmt_{seed:08X}_{last_position}.bin"))
    }

    /// 同じSeed・終端のキャッシュが利用中でも上書きしない新しい保存先を返す。
    pub(crate) fn unique_path(&self, seed: u32, last_position: i64) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        self.directory.join(format!(
            "sfmt_{seed:08X}_{last_position}_{stamp}_{}_{}.bin",
            std::process::id(),
            CACHE_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    pub(crate) fn lease_cache_file(
        &self,
        seed: u32,
        last_position: i64,
    ) -> (PathBuf, Arc<CacheFileLease>) {
        let registry = CACHE_FILE_LEASES.get_or_init(|| Mutex::new(Default::default()));
        let mut registry = registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry.retain(|_, lease| lease.strong_count() > 0);
        let stable_path = self.path(seed, last_position);
        let path = if registry.get(&stable_path).and_then(Weak::upgrade).is_some() {
            self.unique_path(seed, last_position)
        } else {
            stable_path
        };
        let lease = Arc::new(CacheFileLease { path: path.clone() });
        registry.insert(path.clone(), Arc::downgrade(&lease));
        (path, lease)
    }

    /// Share ownership for concurrent users of the same cache file.
    #[cfg(test)]
    pub(crate) fn lease_file(&self, path: &Path) -> Arc<CacheFileLease> {
        let registry = CACHE_FILE_LEASES.get_or_init(|| Mutex::new(Default::default()));
        let mut registry = registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry.retain(|_, lease| lease.strong_count() > 0);
        if let Some(lease) = registry.get(path).and_then(Weak::upgrade) {
            return lease;
        }
        let lease = Arc::new(CacheFileLease {
            path: path.to_path_buf(),
        });
        registry.insert(path.to_path_buf(), Arc::downgrade(&lease));
        lease
    }

    pub(crate) fn create_temporary_file(
        &self,
        seed: u32,
        last_position: i64,
    ) -> AppResult<(PathBuf, Arc<CacheFileLease>, File)> {
        self.ensure_directory()?;
        for _ in 0..32 {
            let path = self.temporary_path(seed, last_position);
            let mut leases = CACHE_FILE_LEASES
                .get_or_init(|| Mutex::new(Default::default()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            leases.retain(|_, lease| lease.strong_count() > 0);
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(file) => {
                    let lease = Arc::new(CacheFileLease { path: path.clone() });
                    leases.insert(path.clone(), Arc::downgrade(&lease));
                    return Ok((path, lease, file));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(AppError::Cache(CacheError::Io {
                        operation: CacheIoOperation::CreateFile,
                        source,
                    }))
                }
            }
        }
        Err(AppError::Cache(CacheError::TempNameExhausted))
    }

    /// Schedule removal of inactive cache files without blocking the UI thread.
    pub(crate) fn clear_unused_files(&self) {
        let directory = self.directory.clone();
        let _ = std::thread::Builder::new()
            .name("sfmt-cache-cleanup".into())
            .spawn(move || clear_unused_cache_files(&directory));
    }

    #[cfg(test)]
    pub(crate) fn clear_unused_files_now(&self) {
        clear_unused_cache_files(&self.directory);
    }

    pub(crate) fn temporary_path(&self, seed: u32, last_position: i64) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        self.directory.join(format!(
            "sfmt_{seed:08X}_{last_position}_{stamp}_{}_{}.tmp",
            std::process::id(),
            CACHE_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    pub(crate) fn is_valid(
        &self,
        path: &Path,
        seed: u32,
        value_count: u64,
        version: u32,
        header_size: u64,
    ) -> AppResult<bool> {
        let Ok(metadata) = fs::metadata(path) else {
            return Ok(false);
        };
        if metadata.len() != header_size + value_count.saturating_mul(8) {
            return Ok(false);
        }
        let mut reader = BufReader::new(File::open(path).map_err(|source| {
            AppError::Cache(CacheError::Io {
                operation: CacheIoOperation::Open,
                source,
            })
        })?);
        let mut magic = [0_u8; 8];
        let mut actual_version = [0_u8; 4];
        let mut actual_seed = [0_u8; 4];
        let mut actual_count = [0_u8; 8];
        reader
            .read_exact(&mut magic)
            .and_then(|_| reader.read_exact(&mut actual_version))
            .and_then(|_| reader.read_exact(&mut actual_seed))
            .and_then(|_| reader.read_exact(&mut actual_count))
            .map_err(|source| {
                AppError::Cache(CacheError::Io {
                    operation: CacheIoOperation::Read,
                    source,
                })
            })?;
        Ok(&magic == CACHE_MAGIC
            && u32::from_le_bytes(actual_version) == version
            && u32::from_le_bytes(actual_seed) == seed
            && u64::from_le_bytes(actual_count) == value_count)
    }
}

fn remove_unleased_cache_files(
    directory: &Path,
    active_paths: &std::collections::HashSet<PathBuf>,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_cache_file = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with("sfmt_") && (name.ends_with(".bin") || name.ends_with(".tmp"))
            });
        if is_cache_file && !active_paths.contains(&path) {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_file_is_removed_after_its_last_lease_is_dropped() {
        let directory = std::env::temp_dir().join(format!(
            "earphone-lost-search-cache-lease-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create test directory");
        let path = directory.join("cache.bin");
        fs::write(&path, b"cache").expect("create test cache");

        let store = CacheStore {
            directory: directory.clone(),
        };
        let first = store.lease_file(&path);
        let second = first.clone();
        drop(first);
        assert!(path.exists());

        drop(second);
        for _ in 0..100 {
            if !path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!path.exists());
        fs::remove_dir(directory).expect("remove test directory");
    }

    #[test]
    fn cleanup_removes_only_unleased_cache_files() {
        let directory = std::env::temp_dir().join(format!(
            "earphone-lost-search-cache-cleanup-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("create test directory");
        let session_directory = directory.join("process-test");
        fs::create_dir_all(&session_directory).expect("create session directory");
        let active_path = session_directory.join("sfmt_active.bin");
        let stale_path = session_directory.join("sfmt_stale.bin");
        let legacy_path = directory.join("sfmt_legacy.bin");
        fs::write(&active_path, b"active").expect("create active cache");
        fs::write(&stale_path, b"stale").expect("create stale cache");
        fs::write(&legacy_path, b"legacy").expect("create legacy cache");

        let store = CacheStore {
            directory: session_directory,
        };
        let _active_lease = store.lease_file(&active_path);
        clear_unused_cache_files(&store.directory);

        assert!(active_path.exists());
        assert!(!stale_path.exists());
        assert!(!legacy_path.exists());
        drop(_active_lease);
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
