//! Generated outputs stay beneath the folder the user chose and never replace unrelated files.

#[cfg(not(target_arch = "wasm32"))]
use std::collections::BTreeMap;
use std::io;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
struct Written {
    bytes: u64,
    hash: blake3::Hash,
}

/// In-memory ownership only. An imported document cannot claim files through a forged sidecar.
#[derive(Clone, Debug, Default)]
pub(crate) struct OwnedOutputs(#[cfg(not(target_arch = "wasm32"))] BTreeMap<(PathBuf, String), Written>);

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// Portable relative names allow subfolders, while refusing traversal and Windows aliases.
pub(crate) fn relative_path(name: &str) -> io::Result<PathBuf> {
    if name.is_empty() || name.starts_with('/') || name.contains(['\\', ':', '\0']) {
        return Err(invalid("export name must be a forward-slash relative path"));
    }
    for component in name.split('/') {
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.ends_with(['.', ' '])
            || component.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
        {
            return Err(invalid("export name contains an empty, parent or ambiguous component"));
        }
        let stem = component.split('.').next().unwrap_or_default().to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$")
            || ["COM", "LPT"]
                .iter()
                .any(|prefix| stem.strip_prefix(prefix).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")))
        {
            return Err(invalid("export name contains a reserved device name"));
        }
    }
    Ok(PathBuf::from(name))
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::io::{Read, Write};

    use cap_std::fs::{Dir, OpenOptions};

    pub(crate) struct OutputDirectory {
        dir: Dir,
        root: PathBuf,
        created: Vec<(String, Written)>,
    }

    impl OutputDirectory {
        pub(crate) fn new(path: &str) -> io::Result<Self> {
            std::fs::create_dir_all(path)?;
            let root = std::fs::canonicalize(path)?;
            let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority())?;
            Ok(Self { dir, root, created: Vec::new() })
        }

        fn unchanged(&self, path: &Path, written: &Written) -> io::Result<bool> {
            // Refuse links, including a replacement link to another file in the output tree.
            if !self.dir.symlink_metadata(path)?.is_file() {
                return Ok(false);
            }
            let file = self.dir.open(path)?;
            if file.metadata()?.len() != written.bytes {
                return Ok(false);
            }
            let mut hash = blake3::Hasher::new();
            let mut reader = file.take(written.bytes.saturating_add(1));
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hash.update(buffer.get(..read).ok_or_else(|| invalid("invalid export read length"))?);
            }
            Ok(hash.finalize() == written.hash)
        }

        pub(crate) fn write(&mut self, name: &str, bytes: &[u8], mut owned: Option<&mut OwnedOutputs>) -> io::Result<()> {
            let relative = relative_path(name)?;
            let key = (self.root.clone(), name.to_string());
            let previous = owned.as_ref().and_then(|outputs| outputs.0.get(&key));
            let replace = match self.dir.symlink_metadata(&relative) {
                Ok(_) => previous.is_some_and(|record| self.unchanged(&relative, record).unwrap_or(false)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => false,
                Err(e) => return Err(e),
            };
            if previous.is_none() && owned.as_ref().is_some_and(|outputs| outputs.0.len() >= 4096) {
                return Err(invalid("this session already owns 4096 generated outputs"));
            }
            if let Some(parent) = relative.parent().filter(|p| !p.as_os_str().is_empty()) {
                self.dir.create_dir_all(parent)?;
            }
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use cap_std::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            if replace {
                // Only an unchanged output created by this session may be replaced. Stage it
                // beside the destination, preserving the old image on a failed encode/write.
                let leaf = relative.file_name().map(|p| p.to_string_lossy()).unwrap_or_default();
                let temporary = relative.with_file_name(photocraft_format::atomic::temp_name(&leaf));
                let mut file = self.dir.open_with(&temporary, &options)?;
                let result = file.write_all(bytes).and_then(|()| photocraft_format::atomic::sync_file(&file.into_std()));
                let result = result.and_then(|()| self.dir.rename(&temporary, &self.dir, &relative));
                if result.is_err() {
                    let _ = self.dir.remove_file(&temporary);
                }
                result?;
            } else {
                // create_new is the collision check: it also refuses dangling links and races.
                let mut file = self.dir.open_with(&relative, &options)?;
                let result = file.write_all(bytes).and_then(|()| photocraft_format::atomic::sync_file(&file.into_std()));
                if result.is_err() {
                    let _ = self.dir.remove_file(&relative);
                }
                result?;
                self.created.push((name.to_string(), Written { bytes: bytes.len() as u64, hash: blake3::hash(bytes) }));
            }
            if let Some(outputs) = owned.as_mut() {
                outputs.0.insert(key, Written { bytes: bytes.len() as u64, hash: blake3::hash(bytes) });
            }
            Ok(())
        }

        /// A failed/cancelled sequence removes only unchanged files it created in this call.
        pub(crate) fn rollback(&mut self) {
            for (name, written) in std::mem::take(&mut self.created) {
                if let Ok(path) = relative_path(&name)
                    && self.unchanged(&path, &written).unwrap_or(false)
                {
                    let _ = self.dir.remove_file(path);
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::OutputDirectory;

#[cfg(target_arch = "wasm32")]
pub(crate) struct OutputDirectory;
#[cfg(target_arch = "wasm32")]
impl OutputDirectory {
    pub(crate) fn new(_: &str) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "generated file outputs need the desktop app"))
    }
    pub(crate) fn write(&mut self, _: &str, _: &[u8], _: Option<&mut OwnedOutputs>) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "generated file outputs need the desktop app"))
    }
    pub(crate) fn rollback(&mut self) {}
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn portable_names_reject_traversal_and_device_aliases() {
        for bad in [
            "",
            "../a.png",
            "a/../../b.png",
            "/a.png",
            "C:/a.png",
            "a\\b.png",
            "a//b.png",
            "./a.png",
            "a./b.png",
            "a /b.png",
            "a\n.png",
            "CON.png",
            "a/LpT1.png",
            "COM¹.jpg",
            "a:name.png",
        ] {
            assert!(relative_path(bad).is_err(), "{bad:?}");
        }
        for good in ["a.png", "icons/a@2x.png", "thumbs/small/icon.png", "éclair.png"] {
            assert!(relative_path(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn existing_files_are_preserved_and_nested_outputs_work() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("existing.png"), b"original").unwrap();
        let mut output = OutputDirectory::new(dir.path().to_str().unwrap()).unwrap();
        assert!(output.write("existing.png", b"replacement", None).is_err());
        assert!(output.write("../escape.png", b"escape", None).is_err());
        output.write("icons/a.png", b"new", None).unwrap();
        assert_eq!(std::fs::read(dir.path().join("existing.png")).unwrap(), b"original");
        assert_eq!(std::fs::read(dir.path().join("icons/a.png")).unwrap(), b"new");
        output.rollback();
        assert!(!dir.path().join("icons/a.png").exists());
        assert!(dir.path().join("existing.png").is_file());
    }

    #[test]
    fn regeneration_requires_unchanged_session_owned_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let mut owned = OwnedOutputs::default();
        let mut output = OutputDirectory::new(dir.path().to_str().unwrap()).unwrap();
        output.write("icon.png", b"one", Some(&mut owned)).unwrap();
        output.write("icon.png", b"two", Some(&mut owned)).unwrap();
        std::fs::write(dir.path().join("icon.png"), b"external edit").unwrap();
        assert!(output.write("icon.png", b"three", Some(&mut owned)).is_err());
        output.rollback();
        assert_eq!(std::fs::read(dir.path().join("icon.png")).unwrap(), b"external edit");
        // A later session cannot claim an old file merely from its name or identical bytes.
        assert!(output.write("icon.png", b"four", Some(&mut OwnedOutputs::default())).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_subfolders_cannot_escape_the_selected_directory() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("icons")).unwrap();
        let mut output = OutputDirectory::new(dir.path().to_str().unwrap()).unwrap();
        assert!(output.write("icons/a.png", b"escape", None).is_err());
        assert!(!outside.path().join("a.png").exists());
        std::fs::write(outside.path().join("existing.png"), b"original").unwrap();
        std::os::unix::fs::symlink(outside.path().join("existing.png"), dir.path().join("linked.png")).unwrap();
        assert!(output.write("linked.png", b"replacement", None).is_err());
        assert_eq!(std::fs::read(outside.path().join("existing.png")).unwrap(), b"original");
    }
}
