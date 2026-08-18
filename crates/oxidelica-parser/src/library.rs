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

/// The variable the language specification names: a list of roots to
/// look through, separated the way the platform separates paths.
pub const MODELICA_PATH: &str = "MODELICAPATH";

/// How deep a library directory is walked. A tree of packages nests as
/// deep as its authors care to nest it, but not without end.
const MAX_TREE_DEPTH: usize = 16;

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
    library_directories(near).into_iter().next()
}

/// Every directory a library may be read from, most deliberate first.
///
/// `MODELICAPATH` may name several, which is what the specification
/// asks of it; the rest of the search contributes at most one.
pub fn library_directories(near: Option<&Path>) -> Vec<PathBuf> {
    fn remember(found: &mut Vec<PathBuf>, path: PathBuf) {
        if holds_models(&path) && !found.contains(&path) {
            found.push(path);
        }
    }
    let mut found: Vec<PathBuf> = Vec::new();
    if let Some(directory) = std::env::var_os(LIBRARY_VARIABLE) {
        remember(&mut found, PathBuf::from(directory));
    }
    if let Some(list) = std::env::var_os(MODELICA_PATH) {
        for root in std::env::split_paths(&list) {
            remember(&mut found, root);
        }
    }
    // A variable that named somewhere settles the matter: mixing what
    // was asked for with what happens to lie around would make a model
    // depend on where it was opened from.
    if found.is_empty() {
        if let Some(one) = climbed_library(near) {
            remember(&mut found, one);
        }
    }
    found
}

/// The `lib` directory the search finds by looking around, when no
/// variable named one.
fn climbed_library(near: Option<&Path>) -> Option<PathBuf> {
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

/// Whether a directory exists and holds at least one Modelica file,
/// at its top or anywhere below it.
fn holds_models(directory: &Path) -> bool {
    !model_files(directory, 0).is_empty()
}

/// Every `.mo` file in a directory tree, in a settled order: a
/// directory's own files before the directories inside it, and each
/// group by name. `package.mo` comes first among its neighbours, since
/// that is where a package says what it is.
fn model_files(directory: &Path, depth: usize) -> Vec<PathBuf> {
    if depth > MAX_TREE_DEPTH {
        return Vec::new();
    }
    let mut here: Vec<PathBuf> = Vec::new();
    let mut below: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(directory).into_iter().flatten().flatten();
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            below.push(path);
        } else if path.extension().is_some_and(|kind| kind == "mo") {
            here.push(path);
        }
    }
    here.sort_by_key(|path| {
        let name = path.file_name().unwrap_or_default().to_owned();
        (name != "package.mo", name)
    });
    below.sort();
    let mut out = here;
    for directory in below {
        out.extend(model_files(&directory, depth + 1));
    }
    out
}

/// Every library file, ready to be passed as context to
/// [`crate::parse_model_with_libraries`]. A library may be one flat
/// directory of `.mo` files or a tree of packages, each file saying
/// with `within` where in the tree it sits. An empty result means no
/// library was found, which is fine for a model that needs none.
pub fn library_sources(near: Option<&Path>) -> Vec<String> {
    library_directories(near)
        .iter()
        .flat_map(|directory| model_files(directory, 0))
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

    /// Tests that touch the environment take turns.
    static ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_library_may_be_a_tree_of_packages() {
        let _guard = ENVIRONMENT
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let sandbox = Sandbox::new("tree");
        let root = sandbox.0.join("lib");
        std::fs::create_dir_all(root.join("Demo/Electrical")).unwrap();
        std::fs::write(
            root.join("Demo/package.mo"),
            "package Demo constant Real pi = 3.141592653589793; end Demo;",
        )
        .unwrap();
        std::fs::write(
            root.join("Demo/Electrical/package.mo"),
            "within Demo; package Electrical end Electrical;",
        )
        .unwrap();
        std::fs::write(
            root.join("Demo/Electrical/Lamp.mo"),
            "within Demo.Electrical; model Lamp parameter Real watts = 60; \
             Real energy(start = 0); equation der(energy) = watts; end Lamp;",
        )
        .unwrap();

        // Every file below the library directory is picked up, and a
        // package says what it is before its members are read.
        let model = sandbox.0.join("use.mo");
        let sources = library_sources(Some(&model));
        assert_eq!(sources.len(), 4, "one per file, `Tiny.mo` included");
        let demo = sources
            .iter()
            .position(|s| s.contains("package Demo"))
            .unwrap();
        let lamp = sources
            .iter()
            .position(|s| s.contains("model Lamp"))
            .unwrap();
        assert!(demo < lamp, "a package comes before what is inside it");

        // And the names those files declare are the ones a model uses.
        let flat = crate::parse_model_with_libraries(
            &sources,
            "model M Demo.Electrical.Lamp lamp(watts = 100); Real turn; \
             equation turn = Demo.pi; end M;",
        )
        .unwrap();
        assert!(flat.components.iter().any(|c| c.name == "lamp.energy"));
        assert_eq!(
            flat.components
                .iter()
                .find(|c| c.name == "lamp.watts")
                .unwrap()
                .binding,
            Some(crate::ast::Expr::Number(100.0))
        );
    }

    #[test]
    fn modelicapath_names_the_roots() {
        let _guard = ENVIRONMENT
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let first = Sandbox::new("path-one");
        let second = Sandbox::new("path-two");
        std::fs::write(second.0.join("lib/Other.mo"), "package Other end Other;").unwrap();
        let list = std::env::join_paths([first.0.join("lib"), second.0.join("lib")]).unwrap();
        std::env::set_var(MODELICA_PATH, &list);
        let found = library_directories(None);
        std::env::remove_var(MODELICA_PATH);
        assert!(found.len() >= 2, "{found:?}");
        assert_eq!(
            found[0].canonicalize().unwrap(),
            first.0.join("lib").canonicalize().unwrap()
        );
        assert_eq!(
            found[1].canonicalize().unwrap(),
            second.0.join("lib").canonicalize().unwrap()
        );
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
