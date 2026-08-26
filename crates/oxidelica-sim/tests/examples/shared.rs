//! What the example runs share: finding an example beside the
//! repository and reading it with the library on the path.

pub(crate) fn example(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name),
    )
    .unwrap()
}

pub(crate) fn with_library(name: &str) -> oxidelica_parser::Model {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = std::fs::read_to_string(root.join("lib/Oxidelica.mo")).unwrap();
    let source = std::fs::read_to_string(root.join("examples").join(name)).unwrap();
    oxidelica_parser::parse_model_with_libraries(&[library], &source).unwrap()
}
