//! Finding the standard library on disk.
//!
//! A model that says `Oxidelica.Electrical.Analog.Basic.Resistor` needs
//! the library that defines it, and where that lives depends on how the
//! tool was obtained: unpacked from a release archive next to the
//! binary, sitting in a checkout, or pointed at by hand. Both the CLI
//! and the IDE look through the same list of places, so a model behaves
//! the same whichever one opens it.

use std::path::{Path, PathBuf};

/// Environment variable naming the library directory outright.
pub const LIBRARY_VARIABLE: &str = "OXIDELICA_LIB";

/// How many levels to climb while looking for a `lib` directory: enough
/// to reach a checkout root from `target/release`.
const CLIMB: usize = 4;

/// The directory holding the library, if one can be found.
///
/// The order is from the most deliberate to the most incidental:
/// `OXIDELICA_LIB`, then a `lib` beside the model being opened or above
/// it, then beside the working directory, then beside the executable -
/// which is where a release archive keeps it.
pub fn library_directory(near: Option<&Path>) -> Option<PathBuf> {
    if let Some(directory) = std::env::var_os(LIBRARY_VARIABLE) {
        let path = PathBuf::from(directory);
        if holds_models(&path) {
            return Some(path);
        }
    }
    let model_directory = near.and_then(|path| path.parent().map(PathBuf::from));
    let executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from));
    [model_directory, std::env::current_dir().ok(), executable]
        .into_iter()
        .flatten()
        .find_map(|start| climb_for_library(&start))
}

/// Look for `lib` in a directory and in the ones above it.
fn climb_for_library(start: &Path) -> Option<PathBuf> {
    let mut directory = start;
    for _ in 0..=CLIMB {
        let candidate = directory.join("lib");
        if holds_models(&candidate) {
            return Some(candidate);
        }
        directory = directory.parent()?;
    }
    None
}

/// Whether a directory exists and holds at least one Modelica file.
fn holds_models(directory: &Path) -> bool {
    std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| entry.path().extension().is_some_and(|e| e == "mo"))
}

/// Every library file, in name order, ready to be passed as context to
/// [`crate::parse_model_with_libraries`]. An empty result means no
/// library was found, which is fine for a model that needs none.
pub fn library_sources(near: Option<&Path>) -> Vec<String> {
    let Some(directory) = library_directory(near) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "mo"))
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway tree with a library in it.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Sandbox {
            let root = std::env::temp_dir().join(format!(
                "oxidelica-lib-{}-{name}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("lib")).unwrap();
            std::fs::create_dir_all(root.join("deep/nested")).unwrap();
            std::fs::write(root.join("lib/Tiny.mo"), "package Tiny end Tiny;").unwrap();
            std::fs::write(root.join("lib/notes.txt"), "not a model").unwrap();
            Sandbox(root)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_library_is_found_above_the_model() {
        let sandbox = Sandbox::new("above");
        let model = sandbox.0.join("deep/nested/model.mo");
        let found = library_directory(Some(&model)).expect("found by climbing");
        assert_eq!(
            found.canonicalize().unwrap(),
            sandbox.0.join("lib").canonicalize().unwrap()
        );
        // And its contents come back, text files left out.
        let sources = library_sources(Some(&model));
        assert_eq!(sources.len(), 1);
        assert!(sources[0].contains("package Tiny"));
    }

    #[test]
    fn a_directory_without_models_is_not_a_library() {
        let sandbox = Sandbox::new("empty");
        let bare = sandbox.0.join("deep");
        std::fs::create_dir_all(bare.join("lib")).unwrap();
        // `deep/lib` exists but holds nothing, so the search keeps going
        // up and lands on the real one.
        let model = bare.join("nested/model.mo");
        let found = library_directory(Some(&model)).expect("keeps climbing");
        assert_eq!(
            found.canonicalize().unwrap(),
            sandbox.0.join("lib").canonicalize().unwrap()
        );
        assert!(!holds_models(&bare.join("lib")));
    }
}
