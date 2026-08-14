use std::{fs, io::ErrorKind, path::Path};

use anyhow::{Context, Result};
use linkup::MachineId;

use crate::{LINKUP_MACHINE_ID_FILE, linkup_file_path};

pub fn load_or_create() -> Result<MachineId> {
    load_or_create_at(&linkup_file_path(LINKUP_MACHINE_ID_FILE))
}

fn load_or_create_at(path: &Path) -> Result<MachineId> {
    match load(path) {
        Ok(id) => Ok(id),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == ErrorKind::NotFound) =>
        {
            create(path)
        }
        Err(error) => Err(error),
    }
}

fn load(path: &Path) -> Result<MachineId> {
    let value = fs::read_to_string(path)
        .with_context(|| format!("Failed to read machine ID from {path:?}"))?;

    value
        .trim()
        .parse()
        .with_context(|| format!("Invalid machine ID stored in {path:?}"))
}

fn create(path: &Path) -> Result<MachineId> {
    let parent = path.parent().expect("Machine ID path should have a parent");

    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create Linkup directory at {parent:?}"))?;

    let id = MachineId::generate();

    fs::write(path, format!("{id}"))
        .with_context(|| format!("Failed to write machine ID to {path:?}"))?;

    Ok(id)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("linkup-machine-id-test-{}", MachineId::generate()));

            fs::create_dir(&path).unwrap();

            Self(path)
        }

        fn join(&self, path: &str) -> PathBuf {
            self.0.join(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn creates_and_reuses_machine_id() {
        let directory = TestDir::new();
        let path = directory.join("nested/machine-id");

        let created = load_or_create_at(&path).unwrap();
        let loaded = load_or_create_at(&path).unwrap();

        assert_eq!(loaded, created);
        assert_eq!(
            fs::read_to_string(path).unwrap().trim(),
            created.to_string()
        );
    }

    #[test]
    fn does_not_replace_an_invalid_machine_id() {
        let directory = TestDir::new();
        let path = directory.join("machine-id");
        fs::write(&path, "not-a-machine-id\n").unwrap();

        let error = load_or_create_at(&path).unwrap_err();

        assert!(error.to_string().contains("Invalid machine ID"));
        assert_eq!(fs::read_to_string(path).unwrap(), "not-a-machine-id\n");
    }
}
