//! Path resolution for Starla
//!
//! Directory resolution priority:
//!
//! **Config directory** (for config.toml):
//! 1. `$CONFIGURATION_DIRECTORY` (systemd)
//! 2. Container: `/config`
//! 3. `$XDG_CONFIG_HOME/starla`
//! 4. Root: `/etc/starla`, non-root: `~/.config/starla`
//!
//! **State directory** (for keys, probe_id, known_hosts):
//! 1. CLI `--state-dir` (via override)
//! 2. `$STATE_DIRECTORY` (systemd)
//! 3. Container: `/state`
//! 4. `$XDG_STATE_HOME/starla`
//! 5. Root: `/var/lib/starla`, non-root: `~/.local/state/starla`
//!
//! **Runtime directory** (for ephemeral databases, caches):
//! 1. `$RUNTIME_DIRECTORY` (systemd)
//! 2. Container: `/run/starla`
//! 3. `$XDG_RUNTIME_DIR/starla`
//! 4. Root: `/run/starla`, non-root: `/tmp/starla-<uid>`

#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Application name used in subdirectories
const APP_NAME: &str = "starla";

/// Detect if running inside a container (Docker, Podman, etc.)
///
/// Checks for:
/// - `container` env var (set by podman, systemd-nspawn)
/// - `/.dockerenv` file (Docker)
/// - `/run/.containerenv` file (Podman)
fn is_container() -> bool {
    std::env::var("container").is_ok()
        || std::path::Path::new("/.dockerenv").exists()
        || std::path::Path::new("/run/.containerenv").exists()
}

/// CLI override for state directory (set once at startup via `set_state_dir`)
static STATE_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// CLI override for runtime directory (set once at startup via
/// `set_runtime_dir`)
static RUNTIME_DIR_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Set the state directory override (from `--state-dir` CLI arg).
/// Must be called before any `state_dir()` calls. Subsequent calls are ignored.
pub fn set_state_dir(path: PathBuf) {
    let _ = STATE_DIR_OVERRIDE.set(path);
}

/// Set the runtime directory override (from `--runtime-dir` CLI arg).
/// Must be called before any `runtime_dir()` calls. Subsequent calls are
/// ignored.
pub fn set_runtime_dir(path: PathBuf) {
    let _ = RUNTIME_DIR_OVERRIDE.set(path);
}

/// Get the configuration directory path
///
/// Priority:
/// 1. `$CONFIGURATION_DIRECTORY` (systemd)
/// 2. Container: `/config`
/// 3. `$XDG_CONFIG_HOME/starla`
/// 4. Root: `/etc/starla`, non-root: `~/.config/starla`
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CONFIGURATION_DIRECTORY") {
        return PathBuf::from(dir);
    }

    if is_container() {
        return PathBuf::from("/config");
    }

    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg_config).join(APP_NAME);
    }

    if is_root() {
        return PathBuf::from("/etc").join(APP_NAME);
    }

    if let Some(home) = home_dir() {
        return home.join(".config").join(APP_NAME);
    }

    PathBuf::from("/etc").join(APP_NAME)
}

/// Get the state directory path (for databases, keys, etc.)
///
/// Priority:
/// 1. CLI override (set via `set_state_dir`)
/// 2. `$STATE_DIRECTORY` (systemd)
/// 3. Container: `/state`
/// 4. `$XDG_STATE_HOME/starla`
/// 5. Root: `/var/lib/starla`, non-root: `~/.local/state/starla`
pub fn state_dir() -> PathBuf {
    if let Some(override_dir) = STATE_DIR_OVERRIDE.get() {
        return override_dir.clone();
    }

    if let Ok(dir) = std::env::var("STATE_DIRECTORY") {
        return PathBuf::from(dir);
    }

    if is_container() {
        return PathBuf::from("/state");
    }

    if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(xdg_state).join(APP_NAME);
    }

    if is_root() {
        return PathBuf::from("/var/lib").join(APP_NAME);
    }

    if let Some(home) = home_dir() {
        return home.join(".local").join("state").join(APP_NAME);
    }

    PathBuf::from("/var/lib").join(APP_NAME)
}

/// Get the runtime directory path (for ephemeral data: databases, caches)
///
/// Priority:
/// 1. CLI override (set via `set_runtime_dir`)
/// 2. `$RUNTIME_DIRECTORY` (systemd)
/// 3. Container: `/run/starla`
/// 4. `$XDG_RUNTIME_DIR/starla`
/// 5. Root: `/run/starla`, non-root: `/tmp/starla-<uid>`
///
/// The last one sits in a directory every user can write to, so it is
/// trusted only once [`ensure_runtime_dir`] has checked it.
pub fn runtime_dir() -> PathBuf {
    resolve_runtime_dir().0
}

/// The runtime directory, and whether it is the fallback in the shared
/// temporary directory (where another user may have created it first).
fn resolve_runtime_dir() -> (PathBuf, bool) {
    if let Some(override_dir) = RUNTIME_DIR_OVERRIDE.get() {
        return (override_dir.clone(), false);
    }

    if let Ok(dir) = std::env::var("RUNTIME_DIRECTORY") {
        return (PathBuf::from(dir), false);
    }

    if is_container() {
        return (PathBuf::from("/run").join(APP_NAME), false);
    }

    if let Ok(xdg_runtime) = std::env::var("XDG_RUNTIME_DIR") {
        return (PathBuf::from(xdg_runtime).join(APP_NAME), false);
    }

    if is_root() {
        return (PathBuf::from("/run").join(APP_NAME), false);
    }

    // Per-user temp directory
    let uid = {
        #[cfg(unix)]
        {
            unsafe { libc::getuid() }
        }
        #[cfg(not(unix))]
        {
            0u32
        }
    };
    (
        std::env::temp_dir().join(format!("{}-{}", APP_NAME, uid)),
        true,
    )
}

/// Create the runtime directory if it is missing, and return it.
///
/// For the `/tmp/starla-<uid>` fallback, the directory is created 0700 and
/// must then be a real directory (not a symlink) owned by this user; a mode
/// other than 0700 is corrected. One that another user created first is an
/// error: whoever owns it could replace the status socket in it, and the
/// tray would show their status, public key included.
pub fn ensure_runtime_dir() -> std::io::Result<PathBuf> {
    let (dir, shared_tmp) = resolve_runtime_dir();
    #[cfg(unix)]
    if shared_tmp {
        ensure_private_dir(&dir, unsafe { libc::geteuid() })?;
        return Ok(dir);
    }
    #[cfg(not(unix))]
    let _ = shared_tmp;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The status socket path, for a client (the tray) to connect to.
///
/// In the `/tmp/starla-<uid>` fallback the directory must be a real one
/// owned by this user that no one else can write to; otherwise this is an
/// error rather than a path to a socket someone else may have put there.
/// Nothing is created or changed.
pub fn trusted_status_socket_path() -> std::io::Result<PathBuf> {
    let (dir, shared_tmp) = resolve_runtime_dir();
    #[cfg(unix)]
    if shared_tmp {
        check_private_dir(&dir, unsafe { libc::geteuid() })?;
    }
    #[cfg(not(unix))]
    let _ = shared_tmp;
    Ok(dir.join("starla.sock"))
}

/// Open `dir` without following a symlink, and require a directory owned
/// by `uid`.
#[cfg(unix)]
fn open_owned_dir(dir: &Path, uid: u32) -> std::io::Result<(std::fs::File, u32)> {
    use std::io::{Error, ErrorKind};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    // O_NOFOLLOW: a symlink in the last component fails (ELOOP) instead of
    // being followed; fstat and fchmod then act on what was opened, which
    // cannot be swapped for something else in between.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(dir)
        .map_err(|e| {
            Error::new(
                e.kind(),
                format!("{}: not a directory of our own: {}", dir.display(), e),
            )
        })?;
    let meta = file.metadata()?;
    if !meta.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{}: not a directory", dir.display()),
        ));
    }
    if meta.uid() != uid {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "{} is owned by uid {}, not by uid {}; remove it or set XDG_RUNTIME_DIR or \
                 --runtime-dir",
                dir.display(),
                meta.uid(),
                uid
            ),
        ));
    }
    Ok((file, meta.mode() & 0o7777))
}

/// Create `dir` 0700 if it is missing, then require it to be a real
/// directory owned by `uid`, and set its mode to 0700 if it is not.
#[cfg(unix)]
fn ensure_private_dir(dir: &Path, uid: u32) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let (file, mode) = open_owned_dir(dir, uid)?;
    if mode != 0o700 {
        file.set_permissions(std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Require `dir` to be a real directory owned by `uid` that no other user
/// can write to. Changes nothing.
#[cfg(unix)]
fn check_private_dir(dir: &Path, uid: u32) -> std::io::Result<()> {
    let (_, mode) = open_owned_dir(dir, uid)?;
    if mode & 0o022 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is writable by others (mode {:o})", dir.display(), mode),
        ));
    }
    Ok(())
}

/// Get the default config file path
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// Get the default probe key path
pub fn probe_key_path() -> PathBuf {
    state_dir().join("probe_key")
}

/// Get the default probe public key path
pub fn probe_pubkey_path() -> PathBuf {
    state_dir().join("probe_key.pub")
}

/// Get the known SSH host keys path
pub fn known_hosts_path() -> PathBuf {
    state_dir().join("known_hosts")
}

/// Get the status socket path (for tray app communication)
pub fn status_socket_path() -> PathBuf {
    runtime_dir().join("starla.sock")
}

/// Get the probe ID file path
pub fn probe_id_path() -> PathBuf {
    state_dir().join("probe_id")
}

/// Path of the file recording the active pause-until state.
pub fn paused_until_path() -> PathBuf {
    state_dir().join("paused_until")
}

/// Read the probe ID from the state directory
///
/// Returns None if the file doesn't exist or can't be parsed
pub fn read_probe_id() -> Option<u32> {
    let path = probe_id_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => content.trim().parse().ok(),
        Err(_) => None,
    }
}

/// Write the probe ID to the state directory
///
/// Creates the state directory if it doesn't exist
pub fn write_probe_id(probe_id: u32) -> std::io::Result<()> {
    let dir = state_dir();
    ensure_dir(&dir)?;
    let path = probe_id_path();
    std::fs::write(&path, probe_id.to_string())
}

/// Check if the current process is running as root (UID 0)
#[cfg(unix)]
fn is_root() -> bool {
    // Safety: getuid() is a simple syscall with no safety concerns
    unsafe { libc::getuid() == 0 }
}

#[cfg(not(unix))]
fn is_root() -> bool {
    false
}

/// Get the user's home directory
fn home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
        return Some(PathBuf::from(home));
    }

    #[cfg(windows)]
    if let Ok(home) = std::env::var("USERPROFILE") {
        return Some(PathBuf::from(home));
    }

    None
}

/// Ensure a directory exists, creating it if necessary
pub fn ensure_dir(path: &PathBuf) -> std::io::Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Ensure the config directory exists
pub fn ensure_config_dir() -> std::io::Result<PathBuf> {
    let dir = config_dir();
    ensure_dir(&dir)?;
    Ok(dir)
}

/// Ensure the state directory exists
pub fn ensure_state_dir() -> std::io::Result<PathBuf> {
    let dir = state_dir();
    ensure_dir(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Mutex to serialize tests that modify environment variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_config_dir_with_env() {
        let _guard = ENV_LOCK.lock().unwrap();

        let original = std::env::var("CONFIGURATION_DIRECTORY").ok();

        unsafe { std::env::set_var("CONFIGURATION_DIRECTORY", "/test/config") };
        assert_eq!(config_dir(), PathBuf::from("/test/config"));

        if let Some(val) = original {
            unsafe { std::env::set_var("CONFIGURATION_DIRECTORY", val) };
        } else {
            unsafe { std::env::remove_var("CONFIGURATION_DIRECTORY") };
        }
    }

    #[test]
    fn test_state_dir_with_env() {
        let _guard = ENV_LOCK.lock().unwrap();

        let original = std::env::var("STATE_DIRECTORY").ok();

        unsafe { std::env::set_var("STATE_DIRECTORY", "/test/state") };
        assert_eq!(state_dir(), PathBuf::from("/test/state"));

        if let Some(val) = original {
            unsafe { std::env::set_var("STATE_DIRECTORY", val) };
        } else {
            unsafe { std::env::remove_var("STATE_DIRECTORY") };
        }
    }

    #[test]
    fn test_xdg_config_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();

        let orig_conf_dir = std::env::var("CONFIGURATION_DIRECTORY").ok();
        let orig_xdg = std::env::var("XDG_CONFIG_HOME").ok();

        unsafe { std::env::remove_var("CONFIGURATION_DIRECTORY") };
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "/home/test/.config") };

        assert_eq!(config_dir(), PathBuf::from("/home/test/.config/starla"));

        if let Some(val) = orig_conf_dir {
            unsafe { std::env::set_var("CONFIGURATION_DIRECTORY", val) };
        }
        if let Some(val) = orig_xdg {
            unsafe { std::env::set_var("XDG_CONFIG_HOME", val) };
        } else {
            unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
        }
    }

    #[test]
    fn test_xdg_state_fallback() {
        let _guard = ENV_LOCK.lock().unwrap();

        let orig_state_dir = std::env::var("STATE_DIRECTORY").ok();
        let orig_xdg = std::env::var("XDG_STATE_HOME").ok();

        unsafe { std::env::remove_var("STATE_DIRECTORY") };
        unsafe { std::env::set_var("XDG_STATE_HOME", "/home/test/.local/state") };

        assert_eq!(state_dir(), PathBuf::from("/home/test/.local/state/starla"));

        if let Some(val) = orig_state_dir {
            unsafe { std::env::set_var("STATE_DIRECTORY", val) };
        }
        if let Some(val) = orig_xdg {
            unsafe { std::env::set_var("XDG_STATE_HOME", val) };
        } else {
            unsafe { std::env::remove_var("XDG_STATE_HOME") };
        }
    }

    #[test]
    fn test_default_file_paths() {
        let config = config_file();
        assert!(config.to_string_lossy().contains("config.toml"));

        let key = probe_key_path();
        assert!(key.to_string_lossy().contains("probe_key"));

        let pid = probe_id_path();
        assert!(pid.to_string_lossy().contains("probe_id"));

        let kh = known_hosts_path();
        assert!(kh.to_string_lossy().contains("known_hosts"));
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(path)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777
    }

    #[cfg(unix)]
    fn our_uid() -> u32 {
        unsafe { libc::geteuid() }
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_private_dir_is_created_0700() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("starla-1000");
        ensure_private_dir(&dir, our_uid()).unwrap();
        assert!(dir.is_dir());
        assert_eq!(mode_of(&dir), 0o700);
        // Once it is there, it passes as it is, and the tray accepts it.
        ensure_private_dir(&dir, our_uid()).unwrap();
        check_private_dir(&dir, our_uid()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn our_own_dir_with_a_looser_mode_is_set_to_0700() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("starla-1000");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        // The tray will not use a directory others can write to.
        assert!(check_private_dir(&dir, our_uid()).is_err());
        ensure_private_dir(&dir, our_uid()).unwrap();
        assert_eq!(mode_of(&dir), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn a_dir_someone_else_owns_is_refused_and_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("starla-1000");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        // Owned by us, so as far as "uid + 1" is concerned someone else
        // created it first.
        let other = our_uid().wrapping_add(1);
        let err = ensure_private_dir(&dir, other).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(check_private_dir(&dir, other).is_err());
        assert_eq!(mode_of(&dir), 0o777);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_refused_and_its_target_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dir = tmp.path().join("starla-1000");
        std::os::unix::fs::symlink(&target, &dir).unwrap();
        assert!(ensure_private_dir(&dir, our_uid()).is_err());
        assert!(check_private_dir(&dir, our_uid()).is_err());
        assert_eq!(mode_of(&target), 0o755);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_in_place_of_the_dir_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("starla-1000");
        std::fs::write(&dir, b"").unwrap();
        assert!(ensure_private_dir(&dir, our_uid()).is_err());
        assert!(check_private_dir(&dir, our_uid()).is_err());
    }

    #[test]
    fn test_probe_id_read_write() {
        let temp_dir =
            std::env::temp_dir().join(format!("starla-test-probe-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let probe_id_file = temp_dir.join("probe_id");
        assert!(!probe_id_file.exists());

        std::fs::write(&probe_id_file, "1014036").unwrap();

        let content = std::fs::read_to_string(&probe_id_file).unwrap();
        let parsed: Option<u32> = content.trim().parse().ok();
        assert_eq!(parsed, Some(1014036));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
