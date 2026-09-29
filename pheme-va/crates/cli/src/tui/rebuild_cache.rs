//! Local, disposable cache of successful adapter builds (never a model cache).
//! Hashing and publication run off the event loop. Entries are immutable and
//! committed by directory rename, so interrupted/concurrent builds cannot expose
//! a partially written executable. A persistent Cargo target is locked through
//! compilation AND publication (Cargo's own lock ends too early for our copy).
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::{fs, io};

use anyhow::{Context, Result};

pub(super) struct Cache {
    workspace: PathBuf,
    root: PathBuf,
    features: Vec<String>,
    compiler: String,
    fingerprint: String,
    entry: PathBuf,
    pub target: PathBuf,
    build_lock: Option<fs::File>,
}

fn digest(bytes: &[u8]) -> String {
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    format!("{:016x}", hash.finish())
}

impl Cache {
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn prepare(
        workspace: &Path,
        root: &Path,
        features: &[&str],
        compiler: &str,
    ) -> Result<Self> {
        let mut features: Vec<_> = features.iter().map(|s| s.to_string()).collect();
        features.sort();
        features.dedup();
        let fingerprint = fingerprint(workspace, &features, compiler)?;
        let entry = root.join("cache-v2").join(digest(fingerprint.as_bytes()));
        #[cfg(unix)]
        let target = root.join("build");
        // Without a portable OS lock on older Rust versions, preserve isolation
        // rather than risk publishing another builder's executable.
        #[cfg(not(unix))]
        let target = root.join(format!("build-{}-{}", std::process::id(), unique_id()));
        Ok(Self {
            workspace: workspace.into(),
            root: root.into(),
            features,
            compiler: compiler.into(),
            fingerprint,
            entry,
            target,
            build_lock: None,
        })
    }

    /// Called only by the background worker. OS locking releases on process
    /// exit, unlike a create-new sentinel that leaves stale locks after crashes.
    pub fn lock_build(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            fs::create_dir_all(&self.root)?;
            let lock = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(self.root.join("build.lock"))?;
            lock_file(&lock, false).context("could not lock adapter build target")?;
            self.build_lock = Some(lock);
        }
        // Inputs or another publisher may have changed while waiting.
        self.fingerprint = fingerprint(&self.workspace, &self.features, &self.compiler)?;
        self.entry = self
            .root
            .join("cache-v2")
            .join(digest(self.fingerprint.as_bytes()));
        Ok(())
    }

    pub fn lookup(&self) -> Option<PathBuf> {
        fs::read_dir(&self.entry)
            .ok()?
            .filter_map(Result::ok)
            .find_map(|entry| self.validate(&entry.path()))
    }

    fn validate(&self, entry: &Path) -> Option<PathBuf> {
        if fs::read_to_string(entry.join("inputs")).ok()? != self.fingerprint {
            return None;
        }
        let binary = entry.join(format!("cli{}", std::env::consts::EXE_SUFFIX));
        let bytes = fs::read(&binary).ok()?;
        if bytes.is_empty() || fs::read_to_string(entry.join("binary-hash")).ok()? != digest(&bytes)
        {
            return None;
        }
        let runtimes: BTreeMap<String, String> =
            toml::from_str(&fs::read_to_string(entry.join("runtimes.toml")).ok()?).ok()?;
        for (path, hash) in runtimes {
            if digest(&fs::read(path).ok()?) != hash {
                return None;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if fs::metadata(&binary).ok()?.permissions().mode() & 0o111 == 0 {
                return None;
            }
        }
        Some(binary)
    }

    pub fn publish(&self, binary: &Path) -> Result<PathBuf> {
        anyhow::ensure!(
            fingerprint(&self.workspace, &self.features, &self.compiler)? == self.fingerprint,
            "adapter build inputs changed during compilation; retry the build"
        );
        if let Some(binary) = self.lookup() {
            return Ok(binary);
        }
        fs::create_dir_all(&self.entry)?;
        let staging = self
            .root
            .join(format!("publish-{}-{}", std::process::id(), unique_id()));
        fs::create_dir(&staging)?;
        let result = (|| {
            let name = format!("cli{}", std::env::consts::EXE_SUFFIX);
            fs::copy(binary, staging.join(&name))?;
            let bytes = fs::read(staging.join(&name))?;
            anyhow::ensure!(!bytes.is_empty(), "adapter executable is empty");
            fs::write(staging.join("binary-hash"), digest(&bytes))?;
            fs::write(staging.join("inputs"), &self.fingerprint)?;
            fs::write(
                staging.join("runtimes.toml"),
                toml::to_string(&self.runtime_files(binary)?)?,
            )?;
            // Unique immutable generations avoid deleting another publisher's
            // entry or overwriting an executable already selected for restart.
            let generation = self
                .entry
                .join(format!("{}-{}", std::process::id(), unique_id()));
            fs::rename(&staging, &generation)?;
            self.validate(&generation)
                .context("published adapter failed cache validation")
        })();
        let _ = fs::remove_dir_all(staging);
        result
    }

    fn runtime_files(&self, binary: &Path) -> Result<BTreeMap<String, String>> {
        let mut files = BTreeMap::new();
        if !self.features.iter().any(|f| f == "zipformer") {
            return Ok(files);
        }
        // cli/build.rs forwards DEP_LITERT_LIB_DIR as an rpath. Read Cargo's
        // retained build-script output instead of guessing litert-sys's cache
        // policy (which can fall back to OUT_DIR or be explicitly overridden).
        let build = binary
            .parent()
            .context("binary has no parent")?
            .join("build");
        for entry in fs::read_dir(build)? {
            let entry = entry?;
            if !entry.file_name().to_string_lossy().starts_with("cli-") {
                continue;
            }
            let Ok(output) = fs::read_to_string(entry.path().join("output")) else {
                continue;
            };
            for dir in output
                .lines()
                .filter_map(|line| line.strip_prefix("cargo:rustc-link-arg=-Wl,-rpath,"))
            {
                let dir = Path::new(dir);
                anyhow::ensure!(
                    dir.is_absolute(),
                    "LiteRT rpath must be absolute for a relocatable cached adapter: {}",
                    dir.display()
                );
                let primary = dir.join(format!("libLiteRt.{}", std::env::consts::DLL_EXTENSION));
                anyhow::ensure!(
                    primary.is_file(),
                    "LiteRT runtime is missing: {}",
                    primary.display()
                );
                for entry in fs::read_dir(dir)? {
                    let entry = entry?;
                    if entry.file_name().to_string_lossy().starts_with("libLiteRt")
                        && entry.path().is_file()
                    {
                        files.insert(
                            entry
                                .path()
                                .to_str()
                                .context("non-UTF8 LiteRT path")?
                                .to_owned(),
                            digest(&fs::read(entry.path())?),
                        );
                    }
                }
            }
        }
        anyhow::ensure!(
            !files.is_empty(),
            "could not verify LiteRT libraries from CLI build-script rpath output"
        );
        Ok(files)
    }
}

#[cfg(unix)]
fn lock_file(file: &fs::File, nonblocking: bool) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn flock(fd: std::ffi::c_int, operation: std::ffi::c_int) -> std::ffi::c_int;
    }
    loop {
        // SAFETY: fd is owned by the live File; flock retains no Rust pointers.
        // LOCK_EX=2, LOCK_NB=4 on the supported Linux/macOS development hosts.
        if unsafe { flock(file.as_raw_fd(), 2 | if nonblocking { 4 } else { 0 }) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn unique_id() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        + u128::from(SERIAL.fetch_add(1, Ordering::Relaxed))
}

fn fingerprint(workspace: &Path, features: &[String], compiler: &str) -> Result<String> {
    let mut files = BTreeMap::new();
    let mut visited = BTreeSet::new();
    // Include non-Rust build inputs too: manifests, lockfile, build scripts,
    // native sources, embedded assets, and added/deleted files.
    tree(workspace, &mut files, &mut visited)?;
    for ancestor in workspace.ancestors() {
        for name in ["rust-toolchain", "rust-toolchain.toml"] {
            optional_file(&ancestor.join(name), &mut files, &mut visited)?;
        }
        for name in ["config", "config.toml"] {
            optional_file(
                &ancestor.join(".cargo").join(name),
                &mut files,
                &mut visited,
            )?;
        }
    }
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")));
    if let Some(home) = cargo_home {
        for name in ["config", "config.toml"] {
            optional_file(&home.join(name), &mut files, &mut visited)?;
        }
    }
    let env = build_environment(workspace, std::env::vars_os());

    // Tool paths alone do not detect an in-place compiler/wrapper replacement.
    for tool in [
        "cargo",
        "rustc",
        "cc",
        "c++",
        "cmake",
        "ninja",
        "pkg-config",
    ] {
        if let Some(path) = resolve(tool, workspace) {
            optional_file(&path, &mut files, &mut visited)?;
        }
    }
    for (key, value) in &env {
        if key != "PATH" {
            let path = workspace.join(value);
            if path.is_file() {
                optional_file(&path, &mut files, &mut visited)?;
            } else if let Some(path) = value.to_str().and_then(|tool| resolve(tool, workspace)) {
                optional_file(&path, &mut files, &mut visited)?;
            }
        }
    }
    // Never persist credentials that may be present in Cargo's environment.
    let environment = digest(format!("{env:?}").as_bytes());
    Ok(format!("adapter-cache-v2\nrelease;locked;cli;explicit-host\n{workspace:?}\n{features:?}\n{compiler}\n{environment}\n{files:?}"))
}

fn volatile_environment(key: &str) -> bool {
    matches!(
        key,
        "CARGO"
            | "CARGO_PRIMARY_PACKAGE"
            | "CARGO_MAKEFLAGS"
            | "CARGO_TARGET_DIR"
            | "CARGO_BUILD_TARGET"
            | "CARGO_BUILD_TARGET_DIR"
            | "RUST_LOG"
            | "RUST_BACKTRACE"
            | "RUST_LIB_BACKTRACE"
    ) || [
        "CARGO_MANIFEST_",
        "CARGO_PKG_",
        "CARGO_FEATURE_",
        "CARGO_BIN_EXE_",
        "CARGO_TERM_",
    ]
    .iter()
    .any(|prefix| key.starts_with(prefix))
        || (key.starts_with("CARGO_TARGET_") && key.ends_with("_RUNNER"))
}

fn build_environment(
    workspace: &Path,
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> BTreeMap<std::ffi::OsString, std::ffi::OsString> {
    let vars: BTreeMap<_, _> = vars.into_iter().collect();
    let mut result = BTreeMap::new();
    for (key, value) in &vars {
        let name = key.to_string_lossy();
        if volatile_environment(&name) {
            continue;
        }
        if !matches!(
            name.as_ref(),
            "PATH" | "HOME" | "SDKROOT" | "MACOSX_DEPLOYMENT_TARGET" | "XDG_CACHE_HOME"
        ) && ![
            "CARGO",
            "RUST",
            "CC",
            "CXX",
            "CFLAGS",
            "CXXFLAGS",
            "CPPFLAGS",
            "LDFLAGS",
            "LD_",
            "DYLD_",
            "LIB",
            "CPATH",
            "C_INCLUDE_PATH",
            "CPLUS_INCLUDE_PATH",
            "INCLUDE",
            "AR",
            "CMAKE",
            "PKG_CONFIG",
            "BINDGEN",
            "WHISPER",
            "LITERT",
            "ORT",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        {
            continue;
        }
        let value = if matches!(
            name.as_ref(),
            "LD_LIBRARY_PATH" | "DYLD_FALLBACK_LIBRARY_PATH"
        ) {
            let paths = std::env::split_paths(value).filter(|path| {
                let rustlib = path.file_name().is_some_and(|n| n == "lib")
                    && path
                        .parent()
                        .and_then(Path::parent)
                        .and_then(Path::file_name)
                        .is_some_and(|n| n == "rustlib");
                !path.as_os_str().is_empty()
                    && !path.starts_with(workspace.join("target"))
                    && !rustlib
                    && !vars
                        .get(std::ffi::OsStr::new("CARGO_TARGET_DIR"))
                        .is_some_and(|target| path.starts_with(workspace.join(target)))
            });
            let value = std::env::join_paths(paths).expect("split paths are valid paths");
            if value.is_empty() {
                continue;
            }
            value
        } else {
            value.clone()
        };
        result.insert(key.clone(), value);
    }
    result
}

/// Cargo must see the same normalized build inputs that the cache fingerprints.
/// In particular, a launcher's target/deps loader paths must not leak into a
/// nested build or be treated as user-supplied native-library search paths.
pub(super) fn configure_command(command: &mut std::process::Command, workspace: &Path) {
    let env = build_environment(workspace, std::env::vars_os());
    for (key, _) in std::env::vars_os() {
        if volatile_environment(&key.to_string_lossy()) {
            command.env_remove(key);
        }
    }
    for key in ["LD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH"] {
        if let Some(value) = env.get(std::ffi::OsStr::new(key)) {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
}

fn resolve(tool: &str, workspace: &Path) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|p| {
            workspace
                .join(p)
                .join(format!("{tool}{}", std::env::consts::EXE_SUFFIX))
        })
        .find(|p| p.is_file())
}

fn optional_file(
    path: &Path,
    files: &mut BTreeMap<PathBuf, String>,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    match fs::read(path) {
        Ok(bytes) => {
            files.insert(path.to_owned(), digest(&bytes));
            // Follow local dependency/patch paths, including dependencies outside
            // the workspace. Also covers Cargo config's `paths` overrides.
            if path
                .file_name()
                .is_some_and(|n| n == "Cargo.toml" || n == "config" || n == "config.toml")
            {
                let text = std::str::from_utf8(&bytes)?;
                let value: toml::Value = toml::from_str(text)?;
                let parent = path.parent().context("input has no parent")?;
                let base = if path
                    .file_name()
                    .is_some_and(|n| n == "config" || n == "config.toml")
                {
                    parent
                        .parent()
                        .context("Cargo config has no base directory")?
                } else {
                    parent
                };
                paths(&value, base, files, visited)?;
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            files.insert(path.into(), "missing".into());
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn paths(
    value: &toml::Value,
    base: &Path,
    files: &mut BTreeMap<PathBuf, String>,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    if let Some(array) = value.as_array() {
        for value in array {
            paths(value, base, files, visited)?;
        }
    }
    if let Some(table) = value.as_table() {
        for (key, value) in table {
            if key == "path" {
                if let Some(path) = value.as_str() {
                    let path = base.join(path);
                    if path.is_dir() {
                        tree(&path, files, visited)?;
                    } else {
                        optional_file(&path, files, visited)?;
                    }
                }
            } else if key == "paths" {
                if let Some(paths) = value.as_array() {
                    for path in paths.iter().filter_map(toml::Value::as_str) {
                        tree(&base.join(path), files, visited)?;
                    }
                }
            } else {
                paths(value, base, files, visited)?;
            }
        }
    }
    Ok(())
}

fn tree(
    path: &Path,
    files: &mut BTreeMap<PathBuf, String>,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("could not fingerprint {}", path.display()))?;
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }
    for entry in fs::read_dir(&canonical)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if entry.file_name() != "target" && entry.file_name() != ".git"
                // Workspace runtime-data directories are not compilation inputs.
                // Keep crates/models and embedded fixtures in the source snapshot.
                && !(canonical.join("Cargo.lock").is_file()
                                    && matches!(entry.file_name().to_str(), Some("models" | "media" | "recordings" | "datasets" | "benchmark-results")))
            {
                tree(&path, files, visited)?;
            }
        } else {
            optional_file(&path, files, visited)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "pheme-cache-{}-{}",
                std::process::id(),
                unique_id()
            ));
            fs::create_dir_all(path.join("crates/cli/src")).unwrap();
            fs::write(path.join("Cargo.toml"), "[workspace]\n").unwrap();
            fs::write(path.join("Cargo.lock"), "lock").unwrap();
            fs::write(path.join("crates/cli/src/main.rs"), "fn main() {}").unwrap();
            Self(path)
        }
        fn cache(&self, features: &[&str], compiler: &str) -> Cache {
            Cache::prepare(
                &self.0,
                &self.0.join("target/tui-adapters"),
                features,
                compiler,
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn launcher_environment_is_normalized_but_build_inputs_are_preserved() {
        let workspace = Path::new("/checkout");
        let vars = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (std::ffi::OsString::from(k), std::ffi::OsString::from(v)))
                .collect::<Vec<_>>()
        };
        let shell = vars(&[
            ("PATH", "/usr/bin"),
            ("LD_LIBRARY_PATH", "/opt/native"),
            ("RUSTFLAGS", "-C opt-level=2"),
        ]);
        let mut launcher = shell.clone();
        launcher.extend(vars(&[("CARGO", "/usr/bin/cargo"), ("CARGO_MANIFEST_DIR", "/checkout/crates/cli"), ("CARGO_PKG_VERSION", "1"), ("CARGO_MAKEFLAGS", "--jobserver-auth=3,4"), ("RUST_LOG", "debug"), ("CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER", "/tmp/runner"), ("LD_LIBRARY_PATH", "/checkout/target/release:/checkout/target/release/deps:/usr/lib/rustlib/x86_64-unknown-linux-gnu/lib:/opt/native")]));
        let expected = build_environment(workspace, shell);
        assert_eq!(expected, build_environment(workspace, launcher.clone()));
        launcher.extend(vars(&[(
            "LD_LIBRARY_PATH",
            "/checkout/target/tui-adapters/build/release/deps:/opt/native",
        )]));
        assert_eq!(expected, build_environment(workspace, launcher.clone()));
        for (key, value) in [
            ("RUSTFLAGS", "-C opt-level=3"),
            ("LD_LIBRARY_PATH", "/other/native"),
            ("CARGO_PROFILE_RELEASE_LTO", "true"),
            ("RUSTC_WRAPPER", "/custom/wrapper"),
            ("XDG_CACHE_HOME", "/other/cache"),
        ] {
            let mut changed = launcher.clone();
            changed.extend(vars(&[(key, value)]));
            assert_ne!(expected, build_environment(workspace, changed), "{key}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn persistent_target_lock_covers_publication_and_releases_on_drop() {
        let fixture = Fixture::new();
        let mut first = fixture.cache(&["whisper"], "compiler");
        let second = fixture.cache(&["zipformer"], "compiler");
        assert_eq!(first.target, second.target);
        first.lock_build().unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(first.root.join("build.lock"))
            .unwrap();
        assert!(lock_file(&lock, true).is_err());
        fs::write(fixture.0.join("new.rs"), "changed").unwrap();
        assert_eq!(first.target, fixture.cache(&["whisper"], "compiler").target);
        drop(first);
        lock_file(&lock, true).unwrap();
    }

    #[test]
    fn runtime_libraries_are_validated_after_copying_the_executable() {
        let fixture = Fixture::new();
        let cache = fixture.cache(&["zipformer"], "compiler");
        let release = cache.target.join("host/release");
        let output = release.join("build/cli-fixture");
        let runtime = cache.target.join("runtime");
        fs::create_dir_all(&output).unwrap();
        fs::create_dir_all(&runtime).unwrap();
        let library = runtime.join(format!("libLiteRt.{}", std::env::consts::DLL_EXTENSION));
        fs::write(&library, "fake runtime").unwrap();
        fs::write(
            output.join("output"),
            format!("cargo:rustc-link-arg=-Wl,-rpath,{}\n", runtime.display()),
        )
        .unwrap();
        let binary = release.join("cli");
        fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
        let published = cache.publish(&binary).unwrap();
        assert_eq!(cache.lookup(), Some(published));
        fs::write(&library, "modified runtime").unwrap();
        assert!(cache.lookup().is_none());
        fs::write(&library, "fake runtime").unwrap();
        assert!(cache.lookup().is_some());
        fs::remove_file(library).unwrap();
        assert!(cache.lookup().is_none());
        // A leftover accelerator/verification file must not disguise a missing
        // primary runtime when republishing after Cargo reports a fresh build.
        fs::write(runtime.join("libLiteRtAccelerator.so"), "plugin").unwrap();
        assert!(cache.publish(&binary).is_err());
    }

    #[test]
    fn runtime_media_is_excluded_but_model_adapter_source_is_not() {
        let fixture = Fixture::new();
        let before = fixture.cache(&["whisper"], "compiler").fingerprint;
        for name in [
            "models",
            "media",
            "recordings",
            "datasets",
            "benchmark-results",
        ] {
            fs::create_dir_all(fixture.0.join(name)).unwrap();
            fs::write(fixture.0.join(name).join("large.bin"), "runtime data").unwrap();
        }
        assert_eq!(before, fixture.cache(&["whisper"], "compiler").fingerprint);
        fs::create_dir_all(fixture.0.join("crates/models/whispercpp")).unwrap();
        fs::write(fixture.0.join("crates/models/whispercpp/lib.rs"), "source").unwrap();
        assert_ne!(before, fixture.cache(&["whisper"], "compiler").fingerprint);
    }

    #[test]
    fn fresh_launcher_reuses_only_complete_unchanged_entries() {
        let fixture = Fixture::new();
        let cache = fixture.cache(&["whisper"], "compiler one");
        assert!(cache.lookup().is_none());
        let binary = cache.publish(&std::env::current_exe().unwrap()).unwrap();
        assert_eq!(
            fixture.cache(&["whisper"], "compiler one").lookup(),
            Some(binary.clone())
        );
        assert!(fixture
            .cache(&["whisper", "zipformer"], "compiler one")
            .lookup()
            .is_none());
        assert!(fixture
            .cache(&["whisper"], "compiler two")
            .lookup()
            .is_none());
        fs::write(&binary, "corrupted").unwrap();
        assert!(cache.lookup().is_none());
        fs::remove_file(binary).unwrap();
        assert!(cache.lookup().is_none());
    }

    #[test]
    fn source_lock_config_and_external_dependency_changes_invalidate() {
        let fixture = Fixture::new();
        let baseline = fixture.cache(&["whisper"], "compiler").fingerprint;
        for name in [
            "crates/cli/src/main.rs",
            "Cargo.lock",
            "build.rs",
            ".cargo/config.toml",
        ] {
            let path = fixture.0.join(name);
            let original = fs::read(&path).ok();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                if name.ends_with(".toml") {
                    "[build]\nrustflags = ['--cfg=test']"
                } else {
                    "changed"
                },
            )
            .unwrap();
            assert_ne!(
                baseline,
                fixture.cache(&["whisper"], "compiler").fingerprint,
                "{name}"
            );
            if let Some(bytes) = original {
                fs::write(path, bytes).unwrap();
            } else {
                fs::remove_file(path).unwrap();
            }
        }
        let external = Fixture::new();
        fs::write(
            fixture.0.join("Cargo.toml"),
            format!("[dependencies.other]\npath = {:?}\n", external.0),
        )
        .unwrap();
        let before = fixture.cache(&["whisper"], "compiler").fingerprint;
        fs::write(external.0.join("crates/cli/src/main.rs"), "changed").unwrap();
        assert_ne!(before, fixture.cache(&["whisper"], "compiler").fingerprint);
    }

    #[test]
    fn feature_order_is_canonical_and_changes_during_build_are_rejected() {
        let fixture = Fixture::new();
        let cache = fixture.cache(&["whisper", "zipformer"], "compiler");
        assert_eq!(
            cache.fingerprint,
            fixture
                .cache(&["zipformer", "whisper", "whisper"], "compiler")
                .fingerprint
        );
        fs::write(fixture.0.join("new.rs"), "new input").unwrap();
        assert!(cache.publish(&std::env::current_exe().unwrap()).is_err());
        assert!(cache.lookup().is_none());
    }
}
