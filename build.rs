// build.rs - Tree-sitter grammar compilation
//
// The tree-sitter grammar crates (tree-sitter-python, tree-sitter-typescript, etc.)
// each include their own build.rs that compiles the grammar C sources via the `cc` crate.
// No additional build-time compilation is needed in this file.
//
// This build.rs exists to:
// 1. Document that grammar compilation is handled by the grammar crates themselves
// 2. Ensure no network downloads occur at build time (all sources are vendored in crates)
// 3. Provide a hook for any future build-time processing
//
// Grammars compiled into the binary (10 languages):
// - Python (tree-sitter-python)
// - TypeScript/TSX (tree-sitter-typescript)
// - JavaScript/JSX (tree-sitter-javascript)
// - Go (tree-sitter-go)
// - Rust (tree-sitter-rust)
// - Java (tree-sitter-java)
// - C# (tree-sitter-c-sharp)
// - C++ (tree-sitter-cpp)
// - Ruby (tree-sitter-ruby)
// - C (tree-sitter-c)
//
// Note: Kotlin support deferred to Phase 7. The tree-sitter-kotlin crate (0.3.x)
// depends on tree-sitter 0.20.x which causes duplicate C symbol linker errors
// when combined with tree-sitter 0.24.x used by all other grammar crates.

fn main() {
    // Grammar crates handle their own C source compilation via their build.rs scripts.
    // Each crate uses the `cc` crate to compile the tree-sitter grammar C files
    // that are bundled within the crate itself (no network downloads).
    //
    // The compiled grammar code is linked into the final binary, ensuring:
    // - No runtime grammar loading is needed
    // - No network access at build time
    // - All 10 language grammars are statically linked

    // Rerun only if this build script changes
    println!("cargo::rerun-if-changed=build.rs");
}
