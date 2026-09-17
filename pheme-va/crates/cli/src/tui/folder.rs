use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub enum FolderEntryKind {
    Parent,
    Directory,
    Wav,
}

#[derive(Clone, Debug)]
pub struct FolderEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: FolderEntryKind,
}

#[derive(Debug)]
pub struct FolderState {
    pub directory: PathBuf,
    pub entries: Vec<FolderEntry>,
    pub selected: usize,
    pub error: Option<String>,
}

impl FolderState {
    pub fn new(directory: PathBuf) -> Self {
        let mut state = Self {
            directory,
            entries: Vec::new(),
            selected: 0,
            error: None,
        };
        state.refresh();
        state
    }

    pub fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;
        self.selected = 0;

        let read_dir = match fs::read_dir(&self.directory) {
            Ok(read_dir) => read_dir,
            Err(error) => {
                self.error = Some(format!(
                    "could not read {}: {error}",
                    self.directory.display()
                ));
                return;
            }
        };

        if self.directory.parent().is_some() {
            self.entries.push(FolderEntry {
                name: "..".to_owned(),
                path: self
                    .directory
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| self.directory.clone()),
                kind: FolderEntryKind::Parent,
            });
        }

        let mut items = read_dir
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if file_type.is_dir() {
                    Some(FolderEntry {
                        name,
                        path,
                        kind: FolderEntryKind::Directory,
                    })
                } else if file_type.is_file()
                    && path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
                {
                    Some(FolderEntry {
                        name,
                        path,
                        kind: FolderEntryKind::Wav,
                    })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        items.sort_by(|left, right| {
            let left_kind = matches!(left.kind, FolderEntryKind::Wav);
            let right_kind = matches!(right.kind, FolderEntryKind::Wav);
            left_kind
                .cmp(&right_kind)
                .then_with(|| {
                    left.name
                        .to_ascii_lowercase()
                        .cmp(&right.name.to_ascii_lowercase())
                })
                .then_with(|| left.name.cmp(&right.name))
        });
        self.entries.extend(items);
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            self.selected = 0;
            return;
        }
        let current = self.selected as isize;
        self.selected =
            (current + delta).clamp(0, self.entries.len().saturating_sub(1) as isize) as usize;
    }

    pub fn selected_entry(&self) -> Option<&FolderEntry> {
        self.entries.get(self.selected)
    }

    pub fn enter_selected(&mut self) -> Result<Option<PathBuf>> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(None);
        };
        match entry.kind {
            FolderEntryKind::Parent | FolderEntryKind::Directory => {
                self.directory = entry.path;
                self.refresh();
                Ok(None)
            }
            FolderEntryKind::Wav => Ok(Some(entry.path)),
        }
    }

    pub fn next_wav(&mut self) -> Option<PathBuf> {
        let start = self.selected.saturating_add(1);
        let index = (start..self.entries.len())
            .chain(0..start.min(self.entries.len()))
            .find(|index| matches!(self.entries[*index].kind, FolderEntryKind::Wav))?;
        self.selected = index;
        self.selected_entry().map(|entry| entry.path.clone())
    }

    pub fn set_directory(&mut self, directory: PathBuf) -> Result<()> {
        let directory = directory
            .canonicalize()
            .with_context(|| format!("could not resolve directory {}", directory.display()))?;
        if !directory.is_dir() {
            anyhow::bail!("{} is not a directory", directory.display());
        }
        self.directory = directory;
        self.refresh();
        Ok(())
    }

    pub fn wav_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.kind, FolderEntryKind::Wav))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temporary_directory() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("pheme-va-folder-test-{suffix}"))
    }

    #[test]
    fn sorts_directories_and_wav_files_and_ignores_other_files() {
        let directory = temporary_directory();
        fs::create_dir_all(directory.join("nested")).unwrap();
        fs::write(directory.join("Bravo.WAV"), b"wav").unwrap();
        fs::write(directory.join("alpha.wav"), b"wav").unwrap();
        fs::write(directory.join("notes.txt"), b"ignore").unwrap();

        let state = FolderState::new(directory.clone());
        let names = state
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["..", "nested", "alpha.wav", "Bravo.WAV"]);
        assert_eq!(state.wav_count(), 2);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn next_wav_wraps_to_the_first_file() {
        let directory = temporary_directory();
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("one.wav"), b"wav").unwrap();
        fs::write(directory.join("two.wav"), b"wav").unwrap();

        let mut state = FolderState::new(directory.clone());
        state.selected = 2;
        assert_eq!(state.next_wav().unwrap().file_name().unwrap(), "one.wav");
        fs::remove_dir_all(directory).unwrap();
    }
}
