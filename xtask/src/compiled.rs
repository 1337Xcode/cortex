//! Which Rust code the product compiles.
//!
//! An invariant tag counts as an enforcing line only where the product builds it. This module
//! parses the root file of every library and binary target with `syn`, follows the `mod`
//! declarations it finds through the files they load, and reports each file reached with the byte
//! ranges inside it that compile only for tests (the rules are in [`crate::cfg`]). A module
//! declared test-only is not followed, so its file is never reached, and a file no root reaches is
//! not compiled at all.
//!
//! Module files are found the way the compiler finds them: `name.rs` or `name/mod.rs` beside a
//! crate root or `mod.rs`, under `parent/` for any other file, under the directories of enclosing
//! inline modules, and at a `#[path]`, including one inside `cfg_attr`, where every file a
//! platform could choose is followed. A module loaded through `#[path]` keeps its own submodules
//! in its own directory. Inside a macro's tokens, which the parser cannot read as items, a
//! test-only attribute covers the tokens up to the next braced block or semicolon at its level
//! (`cfg_if!` branches, `#[test]` functions in `macro_rules!` bodies); modules that only a macro
//! declares are not followed.

use std::collections::BTreeSet;
use std::fs;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::cfg::{self, ModulePaths};

/// A source file the product compiles, with the parts of it that compile only for tests.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CompiledFile {
    /// Path of the file, as found from its root.
    pub(crate) path: PathBuf,
    /// Byte ranges of the file that compile only for tests.
    pub(crate) test_ranges: Vec<Range<usize>>,
}

/// A `mod name;` declaration whose file the walk must load.
#[derive(Debug, PartialEq, Eq)]
struct ModuleDeclaration {
    /// The module name, without a raw-identifier prefix.
    name: String,
    /// The `#[path]` values that may choose its file.
    paths: ModulePaths,
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

/// Every file the targets rooted at `roots` compile, sorted by path.
///
/// # Errors
///
/// Fails when a file cannot be read or parsed, a `cfg` predicate is malformed, or a declared
/// module has no file or two.
pub(crate) fn compiled_files(roots: &[PathBuf]) -> anyhow::Result<Vec<CompiledFile>> {
    let mut pending = Vec::new();
    for root in roots {
        pending.push((normalize(root), folder_of(&normalize(root))?));
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
            pending.extend(locate(&file, &module_dir, declaration)?);
        }
        files.push(CompiledFile {
            path: file,
            test_ranges: scan.test_ranges,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

/// The folder holding `path`.
fn folder_of(path: &Path) -> anyhow::Result<PathBuf> {
    path.parent()
        .map(Path::to_path_buf)
        .with_context(|| format!("finding the folder of {}", path.display()))
}

/// `path` with `.` components dropped and each `..` folded into the folder before it, without
/// touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normal.pop() {
                    normal.push(component);
                }
            }
            other => normal.push(other),
        }
    }
    normal
}

/// Every file a declaration may load, each with the directory its own submodules live in.
fn locate(
    file: &Path,
    module_dir: &Path,
    declaration: &ModuleDeclaration,
) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    let inline_dir = declaration
        .inline_dirs
        .iter()
        .fold(module_dir.to_path_buf(), |dir, component| {
            dir.join(component)
        });
    let path_base = if declaration.inline_dirs.is_empty() {
        folder_of(file)?
    } else {
        inline_dir.clone()
    };
    let mut found = Vec::new();
    for path in &declaration.paths.paths {
        let loaded = normalize(&path_base.join(path));
        if loaded.is_file() {
            let own_dir = folder_of(&loaded)?;
            found.push((loaded, own_dir));
        }
    }
    if !declaration.paths.is_unconditional {
        let own_dir = normalize(&inline_dir.join(&declaration.name));
        let flat = normalize(&inline_dir.join(format!("{}.rs", declaration.name)));
        let nested = own_dir.join("mod.rs");
        match (flat.is_file(), nested.is_file()) {
            (true, true) => bail!(
                "module `{}` declared in {} has two files, {} and {}",
                declaration.name,
                file.display(),
                flat.display(),
                nested.display()
            ),
            (true, false) => found.push((flat, own_dir)),
            (false, true) => found.push((nested, own_dir)),
            (false, false) => {}
        }
    }
    if found.is_empty() {
        bail!(
            "module `{}` declared in {} has no file",
            declaration.name,
            file.display()
        );
    }
    Ok(found)
}

/// The test-only ranges and module declarations of one file's text.
fn scan(text: &str) -> anyhow::Result<FileScan> {
    let offset = skipped_prefix_len(text);
    let body = text.get(offset..).unwrap_or_default();
    let file: syn::File = syn::parse_str(body)?;
    if cfg::is_test_only(&file.attrs)? {
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

/// The attribute starting at `index` in a macro's token trees, if one does: the index after its
/// bracketed content, and whether it makes what follows test-only. Content that is not a valid
/// attribute (macro syntax such as `#[$meta]`) is never test-only.
fn attribute_in_tokens(trees: &[TokenTree], index: usize) -> Option<(usize, bool)> {
    let TokenTree::Punct(pound) = trees.get(index)? else {
        return None;
    };
    if pound.as_char() != '#' {
        return None;
    }
    let mut next = index + 1;
    if matches!(trees.get(next), Some(TokenTree::Punct(bang)) if bang.as_char() == '!') {
        next += 1;
    }
    let TokenTree::Group(group) = trees.get(next)? else {
        return None;
    };
    if group.delimiter() != Delimiter::Bracket {
        return None;
    }
    let is_test = syn::parse2::<syn::Meta>(group.stream())
        .ok()
        .and_then(|meta| cfg::is_test_only_meta(&meta).ok())
        .unwrap_or(false);
    Some((next + 1, is_test))
}

/// The index after the element that attributes ending at `from` sit on, in a macro's token trees:
/// through the next braced block or semicolon, or up to the next comma, at this level.
fn element_end(trees: &[TokenTree], from: usize) -> usize {
    let mut index = from;
    while let Some((after, _)) = attribute_in_tokens(trees, index) {
        index = after;
    }
    while let Some(tree) = trees.get(index) {
        match tree {
            TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => return index + 1,
            TokenTree::Punct(punct) if punct.as_char() == ';' => return index + 1,
            TokenTree::Punct(punct) if punct.as_char() == ',' => return index,
            _ => index += 1,
        }
    }
    index
}

/// One branch of a `cfg_if!`: the value of its own predicate (`None` for the final `else`) and
/// its braced block.
type CfgIfBranch = (Option<cfg::Truth>, proc_macro2::Group);

/// The branches of a `cfg_if!` body: `if #[cfg(...)] { ... }`, then any number of
/// `else if #[cfg(...)] { ... }`, then an optional `else { ... }`. `None` when the tokens have
/// another shape or a predicate cannot be read, so the generic macro scan handles them.
fn cfg_if_branches(tokens: TokenStream) -> Option<Vec<CfgIfBranch>> {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let is_word = |index: usize, word: &str| matches!(trees.get(index), Some(TokenTree::Ident(ident)) if ident == word);
    let braced = |index: usize| match trees.get(index) {
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
            Some(group.clone())
        }
        _ => None,
    };
    let mut branches = Vec::new();
    let mut index = 0;
    loop {
        let has_pound =
            matches!(trees.get(index + 1), Some(TokenTree::Punct(pound)) if pound.as_char() == '#');
        if !is_word(index, "if") || !has_pound {
            return None;
        }
        let Some(TokenTree::Group(attribute)) = trees.get(index + 2) else {
            return None;
        };
        if attribute.delimiter() != Delimiter::Bracket {
            return None;
        }
        let meta = syn::parse2::<syn::Meta>(attribute.stream()).ok()?;
        let truth = cfg::cfg_truth(&meta).ok()??;
        branches.push((Some(truth), braced(index + 3)?));
        index += 4;
        if index == trees.len() {
            return Some(branches);
        }
        if !is_word(index, "else") {
            return None;
        }
        index += 1;
        if !is_word(index, "if") {
            branches.push((None, braced(index)?));
            return (index + 1 == trees.len()).then_some(branches);
        }
    }
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
        match cfg::is_test_only(attributes) {
            Ok(false) => true,
            Ok(true) => {
                self.record(span.byte_range());
                false
            }
            Err(error) => {
                self.error.get_or_insert(error);
                false
            }
        }
    }

    /// Records a parser byte range, shifted back to the file's own offsets.
    fn record(&mut self, range: Range<usize>) {
        self.scan
            .test_ranges
            .push(range.start + self.offset..range.end + self.offset);
    }

    /// Records the elements under test-only attributes inside a macro's tokens, at every level.
    fn scan_macro_tokens(&mut self, tokens: TokenStream) {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        let mut index = 0;
        while let Some(tree) = trees.get(index) {
            if let Some((after, true)) = attribute_in_tokens(&trees, index) {
                let end = element_end(&trees, after);
                if let Some(last) = trees.get(end.saturating_sub(1)) {
                    let range = tree.span().byte_range().start..last.span().byte_range().end;
                    self.record(range);
                }
                index = end.max(index + 1);
                continue;
            }
            if let TokenTree::Group(group) = tree {
                self.scan_macro_tokens(group.stream());
            }
            index += 1;
        }
    }

    /// Records the `cfg_if!` branches that compile only for tests. A branch holds when its own
    /// predicate does and every earlier one does not; the final `else` holds when none does.
    /// Branches that may compile are scanned like any macro body.
    fn scan_cfg_if(&mut self, branches: Vec<CfgIfBranch>) {
        let mut none_before = cfg::Truth::True;
        for (predicate, block) in branches {
            let holds = match predicate {
                Some(truth) => {
                    let holds = none_before.and(truth);
                    none_before = none_before.and(truth.not());
                    holds
                }
                None => none_before,
            };
            if holds == cfg::Truth::False {
                self.record(block.span().byte_range());
            } else {
                self.scan_macro_tokens(block.stream());
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
        let paths = match cfg::module_paths(&node.attrs) {
            Ok(paths) => paths,
            Err(error) => {
                self.error.get_or_insert(error);
                return;
            }
        };
        let name = node.ident.unraw().to_string();
        if node.content.is_some() {
            let dir = paths
                .paths
                .first()
                .filter(|_| paths.is_unconditional)
                .cloned()
                .unwrap_or_else(|| name.clone());
            self.inline_dirs.push(dir);
            visit::visit_item_mod(self, node);
            self.inline_dirs.pop();
        } else {
            self.scan.modules.push(ModuleDeclaration {
                name,
                paths,
                inline_dirs: self.inline_dirs.clone(),
            });
        }
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        let is_cfg_if = node
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "cfg_if");
        let branches = if is_cfg_if {
            cfg_if_branches(node.tokens.clone())
        } else {
            None
        };
        match branches {
            Some(branches) => self.scan_cfg_if(branches),
            None => self.scan_macro_tokens(node.tokens.clone()),
        }
        visit::visit_macro(self, node);
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

    /// A temporary folder holding `files`, removed when dropped.
    struct Tree(PathBuf);

    impl Tree {
        /// Writes each `(relative path, text)` pair under a new temporary folder.
        fn new(label: &str, files: &[(&str, &str)]) -> anyhow::Result<Self> {
            let root = std::env::temp_dir().join(format!(
                "xtask-compiled-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
            for (relative, text) in files {
                let path = root.join(relative);
                fs::create_dir_all(folder_of(&path)?)?;
                fs::write(path, text)?;
            }
            Ok(Self(root))
        }

        /// The compiled files reached from `roots`, relative to the folder, with `/`.
        fn walk(&self, roots: &[&str]) -> anyhow::Result<Vec<String>> {
            let roots: Vec<PathBuf> = roots.iter().map(|root| self.0.join(root)).collect();
            Ok(compiled_files(&roots)?
                .iter()
                .map(|file| {
                    file.path
                        .strip_prefix(&self.0)
                        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default()
                })
                .collect())
        }
    }

    impl Drop for Tree {
        /// Removes the folder; a failure only leaves a temp folder behind.
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
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

    /// Inside macro tokens, a test-only attribute covers the next braced block or item, and
    /// nothing after it.
    #[test]
    fn test_attributes_inside_macros_are_found() -> anyhow::Result<()> {
        let source = r#"
            cfg_if::cfg_if! { if #[cfg(test)] { fn f() { /* T1 */ } } else { fn f() { /* P1 */ } } }
            macro_rules! m { () => { #[test] fn t() { /* T2 */ } fn p() { /* P2 */ } }; }
            fn caller() { run!(#[cfg(test)] { /* T3 */ }, { /* P3 */ }); }
            run!(#[$meta:meta] { /* P4 */ });
            cfg_if::cfg_if! { if #[cfg(not(test))] { fn f() { /* N1 */ } } else { fn f() { /* N2 */ } } }
            cfg_if! { if #[cfg(unix)] { fn g() { /* U1 */ } } else if #[cfg(test)] { fn g() { /* U2 */ } } else { fn g() { /* U3 */ } } }
        "#;
        // Branch values with `test` off: N1 holds (not(test)); N2 needs test, so never; U1 and U3
        // depend on the platform; U2 needs test and not unix, so never.
        let expected = [
            ("/* N1 */", false),
            ("/* N2 */", true),
            ("/* U1 */", false),
            ("/* U2 */", true),
            ("/* U3 */", false),
            ("/* T1 */", true),
            ("/* P1 */", false),
            ("/* T2 */", true),
            ("/* P2 */", false),
            ("/* T3 */", true),
            ("/* P3 */", false),
            ("/* P4 */", false),
        ];
        for (marker, is_test) in expected {
            assert_eq!(is_marked(source, marker)?, is_test, "{marker}");
        }
        Ok(())
    }

    /// An inner `#![cfg(test)]` covers the whole file or the item that holds it, and neither a
    /// shebang line nor a byte-order mark shifts the reported ranges.
    #[test]
    fn inner_attributes_shebangs_and_byte_order_marks() -> anyhow::Result<()> {
        assert!(is_marked("#![cfg(test)]\nfn f() { /* X */ }\n", "/* X */")?);
        let source = "#!/usr/bin/env run\nfn f() { #![cfg(test)] /* X */ }\nfn g() { /* Y */ }\n";
        assert!(is_marked(source, "/* X */")?);
        assert!(!is_marked(source, "/* Y */")?);
        let marked = "\u{feff}#[cfg(test)] fn f() { /* X */ }\nfn g() { /* Y */ }\n";
        assert!(is_marked(marked, "/* X */")?);
        assert!(!is_marked(marked, "/* Y */")?);
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
        let tree = Tree::new(
            "walk",
            &[
                (
                    "src/lib.rs",
                    "mod real;\n#[cfg(test)] mod tests;\n#[cfg(test)] #[path = \"support.rs\"] mod support;\nmod outer { #[cfg(test)] mod inner; mod deep; }\n#[path = \"elsewhere/moved.rs\"] mod moved;\n#[path = \"../shared/common.rs\"] mod common;\nmod nest { #[path = \"custom.rs\"] mod inner; }\n#[path = \"alt\"] mod m { mod x; }\nmod r#match;\n",
                ),
                ("src/real.rs", "mod child;\n"),
                ("src/real/child.rs", ""),
                ("src/tests.rs", ""),
                ("src/support.rs", ""),
                ("src/outer/inner.rs", ""),
                ("src/outer/deep/mod.rs", ""),
                ("src/elsewhere/moved.rs", "mod beside;\n"),
                ("src/elsewhere/beside.rs", ""),
                ("shared/common.rs", ""),
                ("src/nest/custom.rs", ""),
                ("src/alt/x.rs", ""),
                ("src/match.rs", ""),
                ("src/bin/tool.rs", "mod helper;\n"),
                ("src/bin/helper.rs", ""),
                ("src/orphan.rs", ""),
            ],
        )?;
        assert_eq!(
            tree.walk(&["src/lib.rs", "src/bin/tool.rs"])?,
            vec![
                "shared/common.rs",
                "src/alt/x.rs",
                "src/bin/helper.rs",
                "src/bin/tool.rs",
                "src/elsewhere/beside.rs",
                "src/elsewhere/moved.rs",
                "src/lib.rs",
                "src/match.rs",
                "src/nest/custom.rs",
                "src/outer/deep/mod.rs",
                "src/real/child.rs",
                "src/real.rs",
            ]
        );
        Ok(())
    }

    /// Every file a `cfg_attr` path can choose is followed, beside the default file when it
    /// exists; a test-only `cfg_attr` path is not.
    #[test]
    fn walk_follows_every_platform_file() -> anyhow::Result<()> {
        let tree = Tree::new(
            "platform",
            &[
                (
                    "src/lib.rs",
                    "#[cfg_attr(windows, path = \"pipe.rs\")]\n#[cfg_attr(unix, path = \"socket.rs\")]\nmod transport;\n#[cfg_attr(unix, path = \"u.rs\")]\nmod m;\n#[cfg_attr(test, path = \"fake.rs\")]\nmod n;\n",
                ),
                ("src/pipe.rs", ""),
                ("src/socket.rs", ""),
                ("src/u.rs", ""),
                ("src/m.rs", ""),
                ("src/fake.rs", ""),
                ("src/n.rs", ""),
            ],
        )?;
        assert_eq!(
            tree.walk(&["src/lib.rs"])?,
            vec![
                "src/lib.rs",
                "src/m.rs",
                "src/n.rs",
                "src/pipe.rs",
                "src/socket.rs",
                "src/u.rs",
            ]
        );
        Ok(())
    }

    /// A declared module with no file, or with both `name.rs` and `name/mod.rs`, is an error.
    #[test]
    fn missing_or_doubled_module_files_are_errors() -> anyhow::Result<()> {
        let missing = Tree::new("missing", &[("src/lib.rs", "mod gone;\n")])?;
        let error = missing
            .walk(&["src/lib.rs"])
            .err()
            .map(|error| format!("{error:#}"));
        assert!(
            error
                .as_deref()
                .is_some_and(|text| text.contains("has no file")),
            "{error:?}"
        );
        let doubled = Tree::new(
            "doubled",
            &[
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", ""),
                ("src/a/mod.rs", ""),
            ],
        )?;
        let error = doubled
            .walk(&["src/lib.rs"])
            .err()
            .map(|error| format!("{error:#}"));
        assert!(
            error
                .as_deref()
                .is_some_and(|text| text.contains("has two files")),
            "{error:?}"
        );
        Ok(())
    }
}
