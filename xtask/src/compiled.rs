//! Which Rust code the product compiles.
//!
//! An invariant tag counts as an enforcing line only where the product builds it. This module
//! parses the root file of every library and binary target with `syn`, follows the `mod`
//! declarations it finds through the files they load, and reports each file reached with the byte
//! ranges inside it that compile only for tests: anything under an attribute named `test` or
//! `bench` (`#[test]`, `#[tokio::test]`), and anything under a `cfg` that cannot hold while `test`
//! is off (`cfg(test)`, `cfg(all(test, unix))`, `cfg(false)`). A module declared that way is not
//! followed, so its file is never reached, and a file no root reaches is not compiled at all.
//!
//! Module files are found the way the compiler finds them: `name.rs` or `name/mod.rs` beside a
//! crate root or `mod.rs`, under `parent/` for any other file, under the directories of enclosing
//! inline modules, and at the `#[path]` given. A module loaded through `#[path]` keeps its own
//! submodules in its own directory. Modules that only a macro declares are not followed.

use std::collections::BTreeSet;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use proc_macro2::Span;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

/// A source file the product compiles, with the parts of it that compile only for tests.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CompiledFile {
    /// Path of the file, as found from its root.
    pub(crate) path: PathBuf,
    /// Byte ranges of the file that compile only for tests.
    pub(crate) test_ranges: Vec<Range<usize>>,
}

/// A `mod name;` declaration whose file the walk must load.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModuleDeclaration {
    /// The module name, without a raw-identifier prefix.
    name: String,
    /// The `#[path]` value on the declaration, if any.
    path: Option<String>,
    /// Directory components of the inline modules around the declaration, outermost first.
    inline_dirs: Vec<String>,
}

/// What one file holds: its test-only ranges and the module files it declares.
#[derive(Debug, Default)]
struct FileScan {
    /// Byte ranges that compile only for tests.
    test_ranges: Vec<Range<usize>>,
    /// Out-of-line modules declared outside test-only code.
    modules: Vec<ModuleDeclaration>,
}

/// The value a `cfg` predicate takes while `test` is off and every other option is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Truth {
    /// Holds in every build.
    True,
    /// Holds in no build without `test`.
    False,
    /// Depends on the platform, a feature or another option.
    Unknown,
}

/// One `cfg` predicate: a boolean literal or an option such as `test`, `unix` or `all(...)`.
enum Predicate {
    /// `true` or `false`.
    Literal(bool),
    /// An option name, `key = "value"` pair or combinator.
    Option(Box<syn::Meta>),
}

impl Parse for Predicate {
    /// Reads a boolean literal or a meta item.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(syn::LitBool) {
            Ok(Self::Literal(input.parse::<syn::LitBool>()?.value))
        } else {
            Ok(Self::Option(Box::new(input.parse()?)))
        }
    }
}

/// Every file the targets rooted at `roots` compile, sorted by path.
///
/// # Errors
///
/// Fails when a file cannot be read or parsed, a `cfg` predicate is malformed, or a declared
/// module has no file.
pub(crate) fn compiled_files(roots: &[PathBuf]) -> anyhow::Result<Vec<CompiledFile>> {
    let mut pending = Vec::new();
    for root in roots {
        let directory = root
            .parent()
            .with_context(|| format!("finding the folder of {}", root.display()))?;
        pending.push((root.clone(), directory.to_path_buf()));
    }
    let mut seen = BTreeSet::new();
    let mut files = Vec::new();
    while let Some((file, module_dir)) = pending.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        let text =
            fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        let scan = scan(&text).with_context(|| format!("parsing {}", file.display()))?;
        for declaration in &scan.modules {
            pending.push(locate(&file, &module_dir, declaration)?);
        }
        files.push(CompiledFile {
            path: file,
            test_ranges: scan.test_ranges,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

/// The file a declaration loads and the directory that file's own submodules live in.
fn locate(
    file: &Path,
    module_dir: &Path,
    declaration: &ModuleDeclaration,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let inline_dir = declaration
        .inline_dirs
        .iter()
        .fold(module_dir.to_path_buf(), |dir, component| {
            dir.join(component)
        });
    if let Some(path) = &declaration.path {
        let base = if declaration.inline_dirs.is_empty() {
            file.parent()
                .with_context(|| format!("finding the folder of {}", file.display()))?
                .to_path_buf()
        } else {
            inline_dir
        };
        let loaded = base.join(path);
        let own_dir = loaded
            .parent()
            .with_context(|| format!("finding the folder of {}", loaded.display()))?
            .to_path_buf();
        return Ok((loaded, own_dir));
    }
    let own_dir = inline_dir.join(&declaration.name);
    let flat = inline_dir.join(format!("{}.rs", declaration.name));
    let nested = own_dir.join("mod.rs");
    let loaded = if flat.is_file() {
        flat
    } else if nested.is_file() {
        nested
    } else {
        bail!(
            "module `{}` declared in {} has no file",
            declaration.name,
            file.display()
        );
    };
    Ok((loaded, own_dir))
}

/// The test-only ranges and module declarations of one file's text.
fn scan(text: &str) -> anyhow::Result<FileScan> {
    let offset = skipped_prefix_len(text);
    let body = text.get(offset..).unwrap_or_default();
    let file: syn::File = syn::parse_str(body)?;
    if is_test_only(&file.attrs)? {
        let whole_file = 0..text.len();
        return Ok(FileScan {
            test_ranges: vec![whole_file],
            modules: Vec::new(),
        });
    }
    let mut collector = Collector {
        offset,
        scan: FileScan::default(),
        inline_dirs: Vec::new(),
        error: None,
    };
    collector.visit_file(&file);
    match collector.error {
        Some(error) => Err(error),
        None => Ok(collector.scan),
    }
}

/// The length of a byte-order mark and a shebang line at the start of `text`, which the parser
/// does not accept; the ranges it reports are shifted back by this much.
fn skipped_prefix_len(text: &str) -> usize {
    let bom_len = if text.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let rest = text.get(bom_len..).unwrap_or_default();
    // `#![` opens an inner attribute, not a shebang.
    let is_shebang = rest
        .strip_prefix("#!")
        .is_some_and(|after| !after.trim_start().starts_with('['));
    if !is_shebang {
        return bom_len;
    }
    bom_len + rest.find('\n').unwrap_or(rest.len())
}

/// Whether attributes make the item they sit on compile only for tests.
fn is_test_only(attributes: &[syn::Attribute]) -> anyhow::Result<bool> {
    for attribute in attributes {
        let path = attribute.path();
        if path.is_ident("cfg") {
            let syn::Meta::List(list) = &attribute.meta else {
                bail!("a cfg attribute without a predicate");
            };
            let predicate: Predicate = syn::parse2(list.tokens.clone())?;
            if evaluate(&predicate)? == Truth::False {
                return Ok(true);
            }
        } else if path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "test" || segment.ident == "bench")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The value of a predicate while `test` is off.
fn evaluate(predicate: &Predicate) -> anyhow::Result<Truth> {
    let meta = match predicate {
        Predicate::Literal(true) => return Ok(Truth::True),
        Predicate::Literal(false) => return Ok(Truth::False),
        Predicate::Option(meta) => meta.as_ref(),
    };
    let list = match meta {
        syn::Meta::Path(path) if path.is_ident("test") => return Ok(Truth::False),
        syn::Meta::Path(_) | syn::Meta::NameValue(_) => return Ok(Truth::Unknown),
        syn::Meta::List(list) => list,
    };
    let operands =
        list.parse_args_with(Punctuated::<Predicate, syn::Token![,]>::parse_terminated)?;
    let values = operands
        .iter()
        .map(evaluate)
        .collect::<anyhow::Result<Vec<_>>>()?;
    if list.path.is_ident("all") {
        Ok(if values.contains(&Truth::False) {
            Truth::False
        } else if values.iter().all(|value| *value == Truth::True) {
            Truth::True
        } else {
            Truth::Unknown
        })
    } else if list.path.is_ident("any") {
        Ok(if values.contains(&Truth::True) {
            Truth::True
        } else if values.iter().all(|value| *value == Truth::False) {
            Truth::False
        } else {
            Truth::Unknown
        })
    } else if list.path.is_ident("not") {
        match values.as_slice() {
            [Truth::True] => Ok(Truth::False),
            [Truth::False] => Ok(Truth::True),
            [Truth::Unknown] => Ok(Truth::Unknown),
            _ => Err(anyhow!("`not` takes exactly one predicate")),
        }
    } else {
        Ok(Truth::Unknown)
    }
}

/// The `#[path = "..."]` value among `attributes`, if there is one.
fn path_attribute(attributes: &[syn::Attribute]) -> Option<String> {
    attributes.iter().find_map(|attribute| {
        let syn::Meta::NameValue(pair) = &attribute.meta else {
            return None;
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = &pair.value
        else {
            return None;
        };
        pair.path.is_ident("path").then(|| value.value())
    })
}

/// Walks one file's syntax tree, recording test-only nodes and the modules declared outside them.
struct Collector {
    /// Bytes stripped from the front of the file before parsing.
    offset: usize,
    /// What has been found so far.
    scan: FileScan,
    /// Directory components of the inline modules the walk is inside.
    inline_dirs: Vec<String>,
    /// The first error met; the walk cannot return one directly.
    error: Option<anyhow::Error>,
}

impl Collector {
    /// Records the node at `span` as test code and returns `false` when its attributes make it
    /// test-only; returns `true` when the walk should go inside it.
    fn enter(&mut self, attributes: &[syn::Attribute], span: Span) -> bool {
        match is_test_only(attributes) {
            Ok(false) => true,
            Ok(true) => {
                let range = span.byte_range();
                self.scan
                    .test_ranges
                    .push(range.start + self.offset..range.end + self.offset);
                false
            }
            Err(error) => {
                self.error.get_or_insert(error);
                false
            }
        }
    }
}

/// Implements the visit methods of every node type that carries attributes: a test-only node is
/// recorded and skipped, any other node is walked into.
macro_rules! gate_on_attributes {
    ($($method:ident: $node:ident),* $(,)?) => {
        $(
            fn $method(&mut self, node: &'ast syn::$node) {
                if self.enter(&node.attrs, node.span()) {
                    visit::$method(self, node);
                }
            }
        )*
    };
}

impl<'ast> Visit<'ast> for Collector {
    gate_on_attributes! {
        visit_arm: Arm,
        visit_const_param: ConstParam,
        visit_expr_array: ExprArray,
        visit_expr_assign: ExprAssign,
        visit_expr_async: ExprAsync,
        visit_expr_await: ExprAwait,
        visit_expr_binary: ExprBinary,
        visit_expr_block: ExprBlock,
        visit_expr_break: ExprBreak,
        visit_expr_call: ExprCall,
        visit_expr_cast: ExprCast,
        visit_expr_closure: ExprClosure,
        visit_expr_const: ExprConst,
        visit_expr_continue: ExprContinue,
        visit_expr_field: ExprField,
        visit_expr_for_loop: ExprForLoop,
        visit_expr_group: ExprGroup,
        visit_expr_if: ExprIf,
        visit_expr_index: ExprIndex,
        visit_expr_infer: ExprInfer,
        visit_expr_let: ExprLet,
        visit_expr_lit: ExprLit,
        visit_expr_loop: ExprLoop,
        visit_expr_macro: ExprMacro,
        visit_expr_match: ExprMatch,
        visit_expr_method_call: ExprMethodCall,
        visit_expr_paren: ExprParen,
        visit_expr_path: ExprPath,
        visit_expr_range: ExprRange,
        visit_expr_raw_addr: ExprRawAddr,
        visit_expr_reference: ExprReference,
        visit_expr_repeat: ExprRepeat,
        visit_expr_return: ExprReturn,
        visit_expr_struct: ExprStruct,
        visit_expr_try: ExprTry,
        visit_expr_try_block: ExprTryBlock,
        visit_expr_tuple: ExprTuple,
        visit_expr_unary: ExprUnary,
        visit_expr_unsafe: ExprUnsafe,
        visit_expr_while: ExprWhile,
        visit_expr_yield: ExprYield,
        visit_field: Field,
        visit_field_pat: FieldPat,
        visit_field_value: FieldValue,
        visit_fn_ptr_variadic: FnPtrVariadic,
        visit_foreign_item_fn: ForeignItemFn,
        visit_foreign_item_macro: ForeignItemMacro,
        visit_foreign_item_static: ForeignItemStatic,
        visit_foreign_item_type: ForeignItemType,
        visit_impl_item_const: ImplItemConst,
        visit_impl_item_fn: ImplItemFn,
        visit_impl_item_macro: ImplItemMacro,
        visit_impl_item_type: ImplItemType,
        visit_item_const: ItemConst,
        visit_item_enum: ItemEnum,
        visit_item_extern_crate: ItemExternCrate,
        visit_item_fn: ItemFn,
        visit_item_foreign_mod: ItemForeignMod,
        visit_item_impl: ItemImpl,
        visit_item_macro: ItemMacro,
        visit_item_static: ItemStatic,
        visit_item_struct: ItemStruct,
        visit_item_trait: ItemTrait,
        visit_item_trait_alias: ItemTraitAlias,
        visit_item_type: ItemType,
        visit_item_union: ItemUnion,
        visit_item_use: ItemUse,
        visit_lifetime_param: LifetimeParam,
        visit_local: Local,
        visit_named_arg: NamedArg,
        visit_pat_guard: PatGuard,
        visit_pat_ident: PatIdent,
        visit_pat_or: PatOr,
        visit_pat_paren: PatParen,
        visit_pat_reference: PatReference,
        visit_pat_rest: PatRest,
        visit_pat_slice: PatSlice,
        visit_pat_struct: PatStruct,
        visit_pat_tuple: PatTuple,
        visit_pat_tuple_struct: PatTupleStruct,
        visit_pat_type: PatType,
        visit_pat_wild: PatWild,
        visit_predicate_lifetime: PredicateLifetime,
        visit_predicate_type: PredicateType,
        visit_receiver: Receiver,
        visit_stmt_macro: StmtMacro,
        visit_trait_item_const: TraitItemConst,
        visit_trait_item_fn: TraitItemFn,
        visit_trait_item_macro: TraitItemMacro,
        visit_trait_item_type: TraitItemType,
        visit_type_array: TypeArray,
        visit_type_fn_ptr: TypeFnPtr,
        visit_type_group: TypeGroup,
        visit_type_impl_trait: TypeImplTrait,
        visit_type_infer: TypeInfer,
        visit_type_macro: TypeMacro,
        visit_type_never: TypeNever,
        visit_type_param: TypeParam,
        visit_type_paren: TypeParen,
        visit_type_path: TypePath,
        visit_type_ptr: TypePtr,
        visit_type_reference: TypeReference,
        visit_type_slice: TypeSlice,
        visit_type_trait_object: TypeTraitObject,
        visit_type_tuple: TypeTuple,
        visit_variadic: Variadic,
        visit_variant: Variant,
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !self.enter(&node.attrs, node.span()) {
            return;
        }
        let path = path_attribute(&node.attrs);
        if node.content.is_some() {
            self.inline_dirs
                .push(path.unwrap_or_else(|| node.ident.unraw().to_string()));
            visit::visit_item_mod(self, node);
            self.inline_dirs.pop();
        } else {
            self.scan.modules.push(ModuleDeclaration {
                name: node.ident.unraw().to_string(),
                path,
                inline_dirs: self.inline_dirs.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether the byte offset of `marker` in `source` falls inside a test-only range.
    fn is_marked(source: &str, marker: &str) -> anyhow::Result<bool> {
        let offset = source
            .find(marker)
            .with_context(|| format!("{marker} is not in the source"))?;
        Ok(scan(source)?
            .test_ranges
            .iter()
            .any(|range| range.contains(&offset)))
    }

    /// Code under `#[test]`, any attribute ending in `test`, and any `cfg` that needs `test` is
    /// test-only; code under `cfg(not(test))` or a `cfg` that may hold without `test` is not.
    #[test]
    fn test_attributes_and_cfg_predicates_are_judged() -> anyhow::Result<()> {
        let source = r#"
            #[cfg(test)] mod a { fn f() { /* A */ } }
            #[cfg(all(test, unix))] mod b { fn f() { /* B */ } }
            #[tokio::test] async fn c() { /* C */ }
            #[test] fn d() { /* D */ }
            #[cfg(not(test))] fn e() { /* E */ }
            #[cfg(any(test, unix))] fn g() { /* G */ }
            #[cfg(false)] fn h() { /* H */ }
            #[cfg(any(test, false))] fn i() { /* I */ }
            #[cfg_attr(test, allow(dead_code))] fn j() { /* J */ }
            #[cfg(feature = "x")] fn k() { /* K */ }
        "#;
        let expected = [
            ("/* A */", true),
            ("/* B */", true),
            ("/* C */", true),
            ("/* D */", true),
            ("/* E */", false),
            ("/* G */", false),
            ("/* H */", true),
            ("/* I */", true),
            ("/* J */", false),
            ("/* K */", false),
        ];
        for (marker, is_test) in expected {
            assert_eq!(is_marked(source, marker)?, is_test, "{marker}");
        }
        Ok(())
    }

    /// A test attribute on a field, variant, match arm or parameter covers that element only, not
    /// the code after it.
    #[test]
    fn attributes_inside_items_cover_their_element_only() -> anyhow::Result<()> {
        let source = r#"
            struct S { #[cfg(test)] hook: u8, /* F1 */ other: u8 }
            enum E { A, #[cfg(test)] B, /* V1 */ C }
            fn m(x: u8) -> u8 { match x { #[cfg(test)] 0 => { /* M0 */ 1 } _ => { /* M1 */ 2 } } }
            fn p(#[cfg(test)] probe: u8, x: u8) { /* P1 */ }
            fn after() { /* AFTER */ }
        "#;
        let expected = [
            ("hook", true),
            ("/* F1 */", false),
            ("B,", true),
            ("/* V1 */", false),
            ("/* M0 */", true),
            ("/* M1 */", false),
            ("probe", true),
            ("/* P1 */", false),
            ("/* AFTER */", false),
        ];
        for (marker, is_test) in expected {
            assert_eq!(is_marked(source, marker)?, is_test, "{marker}");
        }
        Ok(())
    }

    /// An inner `#![cfg(test)]` covers the whole file or the item that holds it, and a shebang
    /// line does not shift the reported ranges.
    #[test]
    fn inner_attributes_and_shebangs() -> anyhow::Result<()> {
        assert!(is_marked("#![cfg(test)]\nfn f() { /* X */ }\n", "/* X */")?);
        let source = "#!/usr/bin/env run\nfn f() { #![cfg(test)] /* X */ }\nfn g() { /* Y */ }\n";
        assert!(is_marked(source, "/* X */")?);
        assert!(!is_marked(source, "/* Y */")?);
        Ok(())
    }

    /// A malformed `cfg` predicate is an error, never a guess.
    #[test]
    fn malformed_cfg_is_an_error() {
        assert!(scan("#[cfg(not(test, unix))] fn f() {}").is_err());
        assert!(scan("#[cfg] fn f() {}").is_err());
    }

    /// The walk loads modules the way the compiler does and never enters test-only declarations.
    #[test]
    fn walk_follows_compiled_modules_only() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "xtask-compiled-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let files = [
            (
                "src/lib.rs",
                "mod real;\n#[cfg(test)] mod tests;\n#[cfg(test)] #[path = \"support.rs\"] mod support;\nmod outer { #[cfg(test)] mod inner; mod deep; }\n#[path = \"elsewhere/moved.rs\"] mod moved;\n",
            ),
            ("src/real.rs", "mod child;\n"),
            ("src/real/child.rs", ""),
            ("src/tests.rs", ""),
            ("src/support.rs", ""),
            ("src/outer/inner.rs", ""),
            ("src/outer/deep/mod.rs", ""),
            ("src/elsewhere/moved.rs", "mod beside;\n"),
            ("src/elsewhere/beside.rs", ""),
            ("src/bin/tool.rs", "mod helper;\n"),
            ("src/bin/helper.rs", ""),
            ("src/orphan.rs", ""),
        ];
        for (relative, text) in files {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, text)?;
        }
        let found = compiled_files(&[root.join("src/lib.rs"), root.join("src/bin/tool.rs")]);
        let listed: Vec<String> = found?
            .iter()
            .map(|file| {
                file.path
                    .strip_prefix(&root)
                    .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default()
            })
            .collect();
        fs::remove_dir_all(&root)?;
        assert_eq!(
            listed,
            vec![
                "src/bin/helper.rs",
                "src/bin/tool.rs",
                "src/elsewhere/beside.rs",
                "src/elsewhere/moved.rs",
                "src/lib.rs",
                "src/outer/deep/mod.rs",
                "src/real/child.rs",
                "src/real.rs",
            ]
        );
        Ok(())
    }
}
