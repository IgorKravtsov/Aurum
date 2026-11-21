//! Node-style file IO: reads/writes plus error texts shaped like Node's
//! (`ENOENT: no such file or directory, open '/x'`), because those texts
//! reach users inside CLI error messages.

use std::io::{self, ErrorKind};
use std::path::Path;

/// `(code, description)` as printed by Node/libuv for the common failures.
fn describe(kind: ErrorKind, raw_os_error: Option<i32>) -> Option<(&'static str, &'static str)> {
    // errno 1 (EPERM) is reported by the OS as PermissionDenied too.
    if raw_os_error == Some(1) {
        return Some(("EPERM", "operation not permitted"));
    }
    Some(match kind {
        ErrorKind::NotFound => ("ENOENT", "no such file or directory"),
        ErrorKind::PermissionDenied => ("EACCES", "permission denied"),
        ErrorKind::AlreadyExists => ("EEXIST", "file already exists"),
        ErrorKind::IsADirectory => ("EISDIR", "illegal operation on a directory"),
        ErrorKind::NotADirectory => ("ENOTDIR", "not a directory"),
        ErrorKind::StorageFull => ("ENOSPC", "no space left on device"),
        ErrorKind::ReadOnlyFilesystem => ("EROFS", "read-only file system"),
        ErrorKind::InvalidInput => ("EINVAL", "invalid argument"),
        _ => return None,
    })
}

/// Node's `error.code` for an IO error, when it is one we recognise.
pub fn error_code(error: &io::Error) -> Option<&'static str> {
    describe(error.kind(), error.raw_os_error()).map(|(code, _)| code)
}

/// Node's `error.message`; `path` is omitted from the text when `None`
/// (Node does so for `read` on a directory).
pub fn error_message(error: &io::Error, syscall: &str, path: Option<&str>) -> String {
    match (describe(error.kind(), error.raw_os_error()), path) {
        (Some((code, text)), Some(path)) => format!("{code}: {text}, {syscall} '{path}'"),
        (Some((code, text)), None) => format!("{code}: {text}, {syscall}"),
        (None, _) => error.to_string(),
    }
}

/// `fs.promises.readFile(path, "utf8")`; the error is Node's message text.
/// Invalid UTF-8 becomes U+FFFD and a BOM is kept, like Node's decoder.
pub fn read_to_string(path: &Path) -> Result<String, io::Error> {
    std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The message Node gives for a failed `readFile`.
pub fn read_error_message(error: &io::Error, path: &str) -> String {
    if error.kind() == ErrorKind::IsADirectory {
        return error_message(error, "read", None);
    }
    error_message(error, "open", Some(path))
}

/// Creates `path` with owner-only permissions (0600) and writes `content`,
/// then re-applies 0600 in case the file already existed with other rights.
pub fn write_private(path: &Path, content: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(content)?;
    set_private_mode(path)
}

/// `chmod 0600` (a no-op where Unix permissions do not exist).
pub fn set_private_mode(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// `mkdir -p` with mode 0700 for the directories it creates.
pub fn create_private_dir_all(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}
