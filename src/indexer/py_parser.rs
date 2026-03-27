use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Parser};

use super::parser::{
    ImportInfo, RefKind, Symbol, SymbolKind, SymbolRef, TraitImpl, Visibility, extract_body,
    find_child_by_kind, line_range, node_text,
};

// ── Parser setup ────────────────────────────────────────────────

fn parse_python(source: &str) -> Result<tree_sitter::Tree, String> {
    std::thread_local! {
        static PARSER: std::cell::RefCell<Option<Parser>> =
            const { std::cell::RefCell::new(None) };
    }
    PARSER.with_borrow_mut(|slot| {
        let parser = slot.get_or_insert_with(|| {
            let mut p = Parser::new();
            if let Err(e) = p.set_language(&tree_sitter_python::LANGUAGE.into()) {
                tracing::error!("Failed to load Python grammar: {e}");
            }
            p
        });
        parser
            .parse(source, None)
            .ok_or_else(|| "Failed to parse Python source".to_string())
    })
}

// ── Public API ──────────────────────────────────────────────────

pub fn parse_py_source(
    source: &str,
    file_path: &str,
) -> Result<(Vec<Symbol>, Vec<TraitImpl>), String> {
    let tree = parse_python(source)?;
    let root = tree.root_node();
    let mut symbols = Vec::new();
    let mut trait_impls = Vec::new();
    extract_symbols(
        &root,
        source,
        file_path,
        None,
        &mut symbols,
        &mut trait_impls,
    );
    Ok((symbols, trait_impls))
}

/// Check if a file path indicates a Python test file.
#[must_use]
pub fn is_test_py_file(path: &str) -> bool {
    let p = std::path::Path::new(path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem.starts_with("test_") || stem.ends_with("_test") {
        return true;
    }
    // conftest.py is pytest infrastructure
    if stem == "conftest" {
        return true;
    }
    path.contains("/tests/") || path.starts_with("tests/")
}

// ── Enum detection ──────────────────────────────────────────────

const ENUM_BASE_CLASSES: &[&str] = &["Enum", "IntEnum", "Flag", "IntFlag", "StrEnum"];

fn is_enum_class(base_classes: &[String]) -> bool {
    base_classes
        .iter()
        .any(|b| ENUM_BASE_CLASSES.contains(&b.as_str()))
}

// ── Visibility ──────────────────────────────────────────────────

fn get_py_visibility(name: &str) -> Visibility {
    if name.starts_with("__") && name.ends_with("__") && name.len() > 4 {
        // Dunder methods are public
        Visibility::Public
    } else if name.starts_with('_') {
        // Single underscore convention + double underscore name mangling
        Visibility::Private
    } else {
        Visibility::Public
    }
}

// ── Docstring extraction ────────────────────────────────────────

fn extract_docstring(node: &Node, source: &str) -> Option<String> {
    // Look for a `block` child, then its first expression_statement
    // containing a string literal
    let block = find_child_by_kind(node, "block")?;
    let mut cursor = block.walk();
    for child in block.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            let mut inner = child.walk();
            for sub in child.children(&mut inner) {
                if sub.kind() == "string" || sub.kind() == "concatenated_string" {
                    let text = node_text(&sub, source);
                    return Some(strip_docstring(&text));
                }
            }
            // First non-comment statement wasn't a string — no docstring
            return None;
        }
        if child.kind() == "comment" {
            continue;
        }
        // Any other statement means no docstring
        return None;
    }
    None
}

fn strip_docstring(raw: &str) -> String {
    // Strip triple quotes (""" or ''')
    let inner = raw
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
        .or_else(|| raw.strip_prefix("'''").and_then(|s| s.strip_suffix("'''")))
        // Single-quoted strings
        .or_else(|| raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(raw);

    // Strip common leading whitespace (dedent)
    let lines: Vec<&str> = inner.lines().collect();
    if lines.len() <= 1 {
        return inner.trim().to_string();
    }

    // Find minimum indentation of non-empty lines (skip first line)
    let min_indent = lines[1..]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    let mut result = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                result.push(trimmed.to_string());
            }
        } else if line.len() >= min_indent {
            result.push(line[min_indent..].trim_end().to_string());
        } else {
            result.push(String::new());
        }
    }

    // Trim trailing empty lines
    while result.last().is_some_and(std::string::String::is_empty) {
        result.pop();
    }

    result.join("\n")
}

// ── Decorator extraction ────────────────────────────────────────

fn extract_decorators(node: &Node, source: &str) -> Option<String> {
    // For decorated_definition, decorators are direct children
    if node.kind() != "decorated_definition" {
        return None;
    }
    let mut attrs = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let text = node_text(&child, source);
            // Strip leading @
            let inner = text.strip_prefix('@').unwrap_or(&text).trim();
            attrs.push(inner.to_string());
        }
    }
    if attrs.is_empty() {
        return None;
    }
    Some(attrs.join(", "))
}

// ── Signature extraction ────────────────────────────────────────

fn get_py_signature(node: &Node, source: &str) -> String {
    let text = node_text(node, source);
    // Signature is everything up to and including the first ':'
    // that's part of the function/class definition (not in a type hint)
    if let Some(block_start) = find_child_by_kind(node, "block") {
        let sig_end = block_start.start_byte() - node.start_byte();
        let raw = &text[..sig_end];
        raw.lines()
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string()
    } else {
        // Fallback: use get_signature from parser.rs (up to '{')
        // For Python, take up to ':' instead
        let sig_end = text.find(':').map_or(text.len(), |i| i + 1);
        text[..sig_end].trim().to_string()
    }
}

// ── Base class extraction ───────────────────────────────────────

fn extract_base_classes(node: &Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();
    let Some(arg_list) = find_child_by_kind(node, "argument_list") else {
        return bases;
    };
    let mut cursor = arg_list.walk();
    for child in arg_list.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                bases.push(node_text(&child, source));
            }
            "attribute" => {
                // e.g., enum.Enum — take last segment
                let text = node_text(&child, source);
                if let Some(last) = text.rsplit('.').next() {
                    bases.push(last.to_string());
                }
            }
            // keyword_argument (e.g., metaclass=ABCMeta) — skip
            _ => {}
        }
    }
    bases
}

// ── Symbol extraction ───────────────────────────────────────────

fn extract_symbols(
    node: &Node,
    source: &str,
    file_path: &str,
    class_name: Option<&str>,
    symbols: &mut Vec<Symbol>,
    trait_impls: &mut Vec<TraitImpl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sym) = extract_function(&child, source, file_path, class_name, None) {
                    symbols.push(sym);
                }
            }
            "class_definition" => {
                extract_class(&child, source, file_path, None, symbols, trait_impls);
            }
            "decorated_definition" => {
                extract_decorated(&child, source, file_path, class_name, symbols, trait_impls);
            }
            "import_statement" | "import_from_statement" => {
                extract_import(&child, source, file_path, symbols);
            }
            "expression_statement" if class_name.is_none() => {
                // Module-level assignments
                extract_module_assignment(&child, source, file_path, None, symbols);
            }
            "type_alias_statement" => {
                if let Some(sym) = extract_type_alias(&child, source, file_path) {
                    symbols.push(sym);
                }
            }
            _ => {}
        }
    }
}

fn extract_function(
    node: &Node,
    source: &str,
    file_path: &str,
    class_name: Option<&str>,
    decorators: Option<String>,
) -> Option<Symbol> {
    let name_node = find_child_by_kind(node, "identifier")?;
    let name = node_text(&name_node, source);
    let (line_start, line_end) = line_range(node);
    let visibility = get_py_visibility(&name);

    Some(Symbol {
        name,
        kind: SymbolKind::Function,
        visibility,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: get_py_signature(node, source),
        doc_comment: extract_docstring(node, source),
        body: Some(extract_body(node, source)),
        details: None,
        attributes: decorators,
        impl_type: class_name.map(String::from),
    })
}

fn extract_class(
    node: &Node,
    source: &str,
    file_path: &str,
    decorators: Option<String>,
    symbols: &mut Vec<Symbol>,
    trait_impls: &mut Vec<TraitImpl>,
) {
    let Some(name_node) = find_child_by_kind(node, "identifier") else {
        return;
    };
    let class_name = node_text(&name_node, source);
    let (line_start, line_end) = line_range(node);
    let base_classes = extract_base_classes(node, source);
    let is_enum = is_enum_class(&base_classes);

    let kind = if is_enum {
        SymbolKind::Enum
    } else {
        SymbolKind::Class
    };

    // Build details from base classes
    let details = if base_classes.is_empty() {
        None
    } else {
        Some(
            base_classes
                .iter()
                .map(|b| format!("extends {b}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    };

    symbols.push(Symbol {
        name: class_name.clone(),
        kind,
        visibility: get_py_visibility(&class_name),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: get_py_signature(node, source),
        doc_comment: extract_docstring(node, source),
        body: Some(extract_body(node, source)),
        details,
        attributes: decorators,
        impl_type: None,
    });

    // Emit TraitImpl for each base class
    for base in &base_classes {
        trait_impls.push(TraitImpl {
            type_name: class_name.clone(),
            trait_name: base.clone(),
            file_path: file_path.to_string(),
            line_start,
            line_end,
        });
    }

    // Recurse into class body
    if let Some(body) = find_child_by_kind(node, "block") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_definition" => {
                    if let Some(sym) =
                        extract_function(&child, source, file_path, Some(&class_name), None)
                    {
                        symbols.push(sym);
                    }
                }
                "decorated_definition" => {
                    extract_decorated(
                        &child,
                        source,
                        file_path,
                        Some(&class_name),
                        symbols,
                        trait_impls,
                    );
                }
                "expression_statement" => {
                    if is_enum {
                        extract_enum_variant(&child, source, file_path, &class_name, symbols);
                    } else {
                        extract_module_assignment(
                            &child,
                            source,
                            file_path,
                            Some(&class_name),
                            symbols,
                        );
                    }
                }
                "class_definition" => {
                    // Nested class
                    extract_class(&child, source, file_path, None, symbols, trait_impls);
                }
                _ => {}
            }
        }
    }
}

fn extract_decorated(
    node: &Node,
    source: &str,
    file_path: &str,
    class_name: Option<&str>,
    symbols: &mut Vec<Symbol>,
    trait_impls: &mut Vec<TraitImpl>,
) {
    let decorators = extract_decorators(node, source);
    let (dec_start, dec_end) = line_range(node);

    // Find the inner definition
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(mut sym) =
                    extract_function(&child, source, file_path, class_name, decorators.clone())
                {
                    // Use the decorated_definition's line range
                    sym.line_start = dec_start;
                    sym.line_end = dec_end;
                    symbols.push(sym);
                }
                return;
            }
            "class_definition" => {
                extract_class(
                    &child,
                    source,
                    file_path,
                    decorators.clone(),
                    symbols,
                    trait_impls,
                );
                // Fix line range for the class symbol (last pushed)
                if let Some(sym) = symbols.last_mut() {
                    sym.line_start = dec_start;
                    sym.line_end = dec_end;
                }
                return;
            }
            _ => {}
        }
    }
}

fn extract_import(node: &Node, source: &str, file_path: &str, symbols: &mut Vec<Symbol>) {
    let text = node_text(node, source);
    let (line_start, line_end) = line_range(node);
    symbols.push(Symbol {
        name: text.clone(),
        kind: SymbolKind::Use,
        visibility: Visibility::Private,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: text,
        doc_comment: None,
        body: None,
        details: None,
        attributes: None,
        impl_type: None,
    });
}

fn extract_module_assignment(
    node: &Node,
    source: &str,
    file_path: &str,
    class_name: Option<&str>,
    symbols: &mut Vec<Symbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let target = match child.kind() {
            // Regular assignment or annotated assignment (dataclass fields)
            "assignment" | "type" => child.child(0),
            _ => continue,
        };
        if let Some(left) = target
            && left.kind() == "identifier"
        {
            let name = node_text(&left, source);
            let (line_start, line_end) = line_range(&child);
            symbols.push(Symbol {
                name: name.clone(),
                kind: SymbolKind::Const,
                visibility: get_py_visibility(&name),
                file_path: file_path.to_string(),
                line_start,
                line_end,
                signature: node_text(&child, source),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: class_name.map(String::from),
            });
        }
    }
}

fn extract_enum_variant(
    node: &Node,
    source: &str,
    file_path: &str,
    enum_name: &str,
    symbols: &mut Vec<Symbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "assignment"
            && let Some(left) = child.child(0)
            && left.kind() == "identifier"
        {
            let name = node_text(&left, source);
            // Skip _ignore_ and similar
            if name.starts_with('_') {
                continue;
            }
            let (line_start, line_end) = line_range(&child);
            symbols.push(Symbol {
                name,
                kind: SymbolKind::EnumVariant,
                visibility: Visibility::Public,
                file_path: file_path.to_string(),
                line_start,
                line_end,
                signature: node_text(&child, source),
                doc_comment: None,
                body: None,
                details: None,
                attributes: None,
                impl_type: Some(enum_name.to_string()),
            });
        }
    }
}

fn extract_type_alias(node: &Node, source: &str, file_path: &str) -> Option<Symbol> {
    // Python 3.12+: type Point = tuple[int, int]
    let name_node = find_child_by_kind(node, "type")?;
    let name = node_text(&name_node, source);
    let (line_start, line_end) = line_range(node);

    Some(Symbol {
        name,
        kind: SymbolKind::TypeAlias,
        visibility: Visibility::Public,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: node_text(node, source),
        doc_comment: None,
        body: Some(extract_body(node, source)),
        details: None,
        attributes: None,
        impl_type: None,
    })
}

// ── Noisy symbols ───────────────────────────────────────────────

const PY_NOISY_SYMBOLS: &[&str] = &[
    // Built-in functions
    "print",
    "len",
    "range",
    "str",
    "int",
    "float",
    "list",
    "dict",
    "set",
    "tuple",
    "bool",
    "type",
    "super",
    "self",
    "cls",
    "isinstance",
    "issubclass",
    "hasattr",
    "getattr",
    "setattr",
    "delattr",
    "open",
    "input",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sorted",
    "reversed",
    "any",
    "all",
    "min",
    "max",
    "sum",
    "abs",
    "round",
    "id",
    "hash",
    "repr",
    "format",
    "next",
    "iter",
    "property",
    "staticmethod",
    "classmethod",
    "object",
    "bytes",
    "bytearray",
    "memoryview",
    "frozenset",
    "complex",
    "divmod",
    "pow",
    "chr",
    "ord",
    "hex",
    "oct",
    "bin",
    "callable",
    // Constants
    "None",
    "True",
    "False",
    // Common exceptions
    "Exception",
    "ValueError",
    "TypeError",
    "KeyError",
    "IndexError",
    "AttributeError",
    "RuntimeError",
    "StopIteration",
    "NotImplementedError",
    "OSError",
    "IOError",
    "FileNotFoundError",
    "ImportError",
    "NameError",
    "ZeroDivisionError",
    "OverflowError",
    // Dunder methods
    "__init__",
    "__str__",
    "__repr__",
    "__eq__",
    "__ne__",
    "__hash__",
    "__len__",
    "__iter__",
    "__next__",
    "__enter__",
    "__exit__",
    "__call__",
    "__getitem__",
    "__setitem__",
    "__delitem__",
    "__contains__",
    "__add__",
    "__sub__",
    "__mul__",
    "__truediv__",
    "__bool__",
    "__int__",
    "__float__",
    "__lt__",
    "__le__",
    "__gt__",
    "__ge__",
    "__new__",
    "__del__",
    "__getattr__",
    "__setattr__",
    "__init_subclass__",
    "__class_getitem__",
    // Common methods on builtins
    "append",
    "extend",
    "insert",
    "remove",
    "pop",
    "clear",
    "index",
    "count",
    "sort",
    "reverse",
    "copy",
    "get",
    "keys",
    "values",
    "items",
    "update",
    "setdefault",
    "add",
    "discard",
    "union",
    "intersection",
    "difference",
    "join",
    "split",
    "strip",
    "lstrip",
    "rstrip",
    "replace",
    "startswith",
    "endswith",
    "upper",
    "lower",
    "title",
    "encode",
    "decode",
    "find",
    "rfind",
    "read",
    "write",
    "close",
    "flush",
    "seek",
    "isdigit",
    "isalpha",
    "isalnum",
    "isspace",
];

fn is_noisy_py_symbol(name: &str) -> bool {
    PY_NOISY_SYMBOLS.contains(&name)
}

// ── Reference extraction ────────────────────────────────────────

struct PyRefContext<'a, S: std::hash::BuildHasher> {
    source: &'a str,
    file_path: &'a str,
    known_symbols: &'a HashSet<String, S>,
    import_map: HashMap<String, ImportInfo>,
}

pub fn extract_py_refs<S: std::hash::BuildHasher>(
    source: &str,
    file_path: &str,
    known_symbols: &HashSet<String, S>,
    repo_path: &std::path::Path,
) -> Result<Vec<SymbolRef>, String> {
    let tree = parse_python(source)?;
    let root = tree.root_node();
    let raw_import_map = build_py_import_map(&root, source);

    // Resolve import specifiers to filesystem paths
    let import_map: HashMap<String, ImportInfo> = raw_import_map
        .into_iter()
        .map(|(name, info)| {
            let resolved =
                super::py_imports::resolve_py_import(&info.qualified_path, file_path, repo_path);
            let qualified_path = resolved.unwrap_or(info.qualified_path);
            (name, ImportInfo { qualified_path })
        })
        .collect();

    let ctx = PyRefContext {
        source,
        file_path,
        known_symbols,
        import_map,
    };
    let mut refs = Vec::new();
    collect_py_refs(&root, &ctx, None, &mut refs);
    Ok(refs)
}

fn build_py_import_map(root: &Node, source: &str) -> HashMap<String, ImportInfo> {
    let mut map = HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                parse_import_names(&child, source, &mut map);
            }
            "import_from_statement" => {
                parse_from_import(&child, source, &mut map);
            }
            _ => {}
        }
    }
    map
}

fn parse_import_names(node: &Node, source: &str, map: &mut HashMap<String, ImportInfo>) {
    // import foo, import foo.bar, import foo as f
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                let full_path = node_text(&child, source);
                // The local name is the first segment: `import foo.bar` → "foo"
                let local = full_path.split('.').next().unwrap_or(&full_path);
                map.insert(
                    local.to_string(),
                    ImportInfo {
                        qualified_path: full_path,
                    },
                );
            }
            "aliased_import" => {
                let name = find_child_by_kind(&child, "identifier").map(|n| node_text(&n, source));
                let module =
                    find_child_by_kind(&child, "dotted_name").map(|n| node_text(&n, source));
                if let (Some(alias), Some(module_path)) = (name, module) {
                    map.insert(
                        alias,
                        ImportInfo {
                            qualified_path: module_path,
                        },
                    );
                }
            }
            _ => {}
        }
    }
}

fn parse_from_import(node: &Node, source: &str, map: &mut HashMap<String, ImportInfo>) {
    // from foo.bar import baz, qux
    // from . import bar
    // from ..utils import helper

    // Extract the module path (may be relative with dots)
    let mut module_path = String::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                if module_path.is_empty() || module_path.chars().all(|c| c == '.') {
                    module_path.push_str(&node_text(&child, source));
                } else {
                    // This is an imported name (not aliased)
                    let name = node_text(&child, source);
                    map.insert(
                        name,
                        ImportInfo {
                            qualified_path: module_path.clone(),
                        },
                    );
                }
            }
            "relative_import" => {
                module_path = node_text(&child, source);
            }
            "identifier" => {
                // Single name import: from foo import bar
                let name = node_text(&child, source);
                if name != "import" && name != "from" {
                    map.insert(
                        name,
                        ImportInfo {
                            qualified_path: module_path.clone(),
                        },
                    );
                }
            }
            "aliased_import" => {
                // from foo import bar as baz
                let alias = child
                    .child_by_field_name("alias")
                    .map(|n| node_text(&n, source));
                let original = find_child_by_kind(&child, "dotted_name")
                    .or_else(|| find_child_by_kind(&child, "identifier"))
                    .map(|n| node_text(&n, source));
                if let Some(local) = alias.or(original) {
                    map.insert(
                        local,
                        ImportInfo {
                            qualified_path: module_path.clone(),
                        },
                    );
                }
            }
            // wildcard_import (from foo import *) — can't resolve names
            _ => {}
        }
    }
}

fn collect_py_refs<S: std::hash::BuildHasher>(
    node: &Node,
    ctx: &PyRefContext<'_, S>,
    class_name: Option<&str>,
    refs: &mut Vec<SymbolRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                let name =
                    find_child_by_kind(&child, "identifier").map(|n| node_text(&n, ctx.source));
                if let Some(fn_name) = name {
                    collect_py_body_refs(&child, &fn_name, class_name, ctx, refs);
                }
            }
            "class_definition" => {
                let type_name =
                    find_child_by_kind(&child, "identifier").map(|n| node_text(&n, ctx.source));
                if let Some(body) = find_child_by_kind(&child, "block") {
                    collect_py_refs(&body, ctx, type_name.as_deref(), refs);
                }
            }
            "decorated_definition" => {
                // Recurse — inner function/class definitions will be matched above
                collect_py_refs(&child, ctx, class_name, refs);
            }
            _ => {}
        }
    }
}

fn collect_py_locals(node: &Node, source: &str) -> HashSet<String> {
    let mut locals = HashSet::new();
    // Collect function parameters
    if let Some(params) = find_child_by_kind(node, "parameters") {
        let mut cursor = params.walk();
        for child in params.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    locals.insert(node_text(&child, source));
                }
                "default_parameter"
                | "typed_parameter"
                | "typed_default_parameter"
                | "list_splat_pattern"
                | "dictionary_splat_pattern" => {
                    if let Some(id) = find_child_by_kind(&child, "identifier") {
                        locals.insert(node_text(&id, source));
                    }
                }
                _ => {}
            }
        }
    }
    // Also collect assignment targets in the body
    if let Some(body) = find_child_by_kind(node, "block") {
        collect_assignment_targets(&body, source, &mut locals);
    }
    locals
}

fn collect_assignment_targets(node: &Node, source: &str, locals: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "expression_statement" => {
                let mut inner = child.walk();
                for sub in child.children(&mut inner) {
                    if (sub.kind() == "assignment" || sub.kind() == "augmented_assignment")
                        && let Some(left) = sub.child(0)
                        && left.kind() == "identifier"
                    {
                        locals.insert(node_text(&left, source));
                    }
                }
            }
            "for_statement" => {
                // `for x in ...:` — x is local
                if let Some(left) = child.child(1)
                    && left.kind() == "identifier"
                {
                    locals.insert(node_text(&left, source));
                }
            }
            _ => {}
        }
    }
}

fn collect_py_body_refs<S: std::hash::BuildHasher>(
    node: &Node,
    fn_name: &str,
    impl_type: Option<&str>,
    ctx: &PyRefContext<'_, S>,
    refs: &mut Vec<SymbolRef>,
) {
    let locals = collect_py_locals(node, ctx.source);
    let mut seen = HashSet::new();
    let mut stack = vec![*node];

    while let Some(n) = stack.pop() {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            match child.kind() {
                "call" => {
                    if let Some(func) = child.child(0) {
                        process_py_call(&func, fn_name, impl_type, ctx, &locals, &mut seen, refs);
                    }
                    stack.push(child);
                }
                "attribute" if n.kind() != "call" => {
                    // Standalone attribute access (not as part of a call)
                    // e.g., self.value, Foo.CLASS_VAR
                    process_py_attribute(&child, fn_name, impl_type, ctx, &locals, &mut seen, refs);
                }
                "identifier" => {
                    let name = node_text(&child, ctx.source);
                    // Type references in annotations
                    let is_type_context = child
                        .parent()
                        .is_some_and(|p| p.kind() == "type" || p.kind() == "generic_type");
                    if is_type_context {
                        try_add_py_ref(
                            &name,
                            &PyRefInfo {
                                source_name: fn_name,
                                target_context: None,
                                kind: RefKind::TypeRef,
                                ref_line: ref_line_from_node(&child),
                            },
                            ctx,
                            &locals,
                            &mut seen,
                            refs,
                        );
                    }
                }
                // Skip nested function/class defs
                "function_definition" | "class_definition" | "decorated_definition" => {}
                _ => {
                    stack.push(child);
                }
            }
        }
    }
}

fn process_py_call<S: std::hash::BuildHasher>(
    func: &Node,
    fn_name: &str,
    impl_type: Option<&str>,
    ctx: &PyRefContext<'_, S>,
    locals: &HashSet<String>,
    seen: &mut HashSet<String>,
    refs: &mut Vec<SymbolRef>,
) {
    let ref_line = ref_line_from_node(func);
    match func.kind() {
        "identifier" => {
            let name = node_text(func, ctx.source);
            try_add_py_ref(
                &name,
                &PyRefInfo {
                    source_name: fn_name,
                    target_context: None,
                    kind: RefKind::Call,
                    ref_line,
                },
                ctx,
                locals,
                seen,
                refs,
            );
        }
        "attribute" => {
            process_py_attribute(func, fn_name, impl_type, ctx, locals, seen, refs);
        }
        _ => {}
    }
}

fn process_py_attribute<S: std::hash::BuildHasher>(
    node: &Node,
    fn_name: &str,
    impl_type: Option<&str>,
    ctx: &PyRefContext<'_, S>,
    locals: &HashSet<String>,
    seen: &mut HashSet<String>,
    refs: &mut Vec<SymbolRef>,
) {
    // attribute node: obj.method
    // child(0) = object, last named child = property identifier
    let obj = node.child(0);
    let prop = node
        .child_by_field_name("attribute")
        .or_else(|| node.child(2));

    let Some(prop) = prop else { return };
    if prop.kind() != "identifier" {
        return;
    }

    let method_name = node_text(&prop, ctx.source);
    let ref_line = ref_line_from_node(node);

    let target_context = obj.and_then(|o| match o.kind() {
        "identifier" => {
            let name = node_text(&o, ctx.source);
            match name.as_str() {
                "self" | "cls" => impl_type.map(String::from),
                _ if name.chars().next().is_some_and(char::is_uppercase) => Some(name),
                _ => None,
            }
        }
        _ => None,
    });

    try_add_py_ref(
        &method_name,
        &PyRefInfo {
            source_name: fn_name,
            target_context: target_context.as_deref(),
            kind: RefKind::Call,
            ref_line,
        },
        ctx,
        locals,
        seen,
        refs,
    );
}

struct PyRefInfo<'a> {
    source_name: &'a str,
    target_context: Option<&'a str>,
    kind: RefKind,
    ref_line: i64,
}

fn ref_line_from_node(node: &Node) -> i64 {
    i64::try_from(node.start_position().row).unwrap_or(0) + 1
}

fn try_add_py_ref<S: std::hash::BuildHasher>(
    name: &str,
    info: &PyRefInfo<'_>,
    ctx: &PyRefContext<'_, S>,
    locals: &HashSet<String>,
    seen: &mut HashSet<String>,
    refs: &mut Vec<SymbolRef>,
) {
    if name.is_empty() || name == info.source_name {
        return;
    }

    let dedup_key = if let Some(tc) = info.target_context {
        format!("{tc}::{name}")
    } else {
        name.to_string()
    };
    if !seen.insert(dedup_key) {
        return;
    }

    if locals.contains(name) {
        return;
    }

    if info.target_context.is_none() && is_noisy_py_symbol(name) {
        return;
    }

    if !ctx.known_symbols.contains(name) {
        return;
    }

    let target_file = ctx
        .import_map
        .get(name)
        .map(|imp| imp.qualified_path.clone());

    refs.push(SymbolRef {
        source_name: info.source_name.to_string(),
        source_file: ctx.file_path.to_string(),
        target_name: name.to_string(),
        kind: info.kind,
        target_file,
        target_context: info.target_context.map(String::from),
        ref_line: Some(info.ref_line),
    });
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_parse_function() {
        let source = r#"
def greet(name: str) -> str:
    """Say hello."""
    return f"Hello, {name}!"
"#;
        let (symbols, _) = parse_py_source(source, "src/utils.py").unwrap();
        let func = symbols.iter().find(|s| s.name == "greet").unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.visibility, Visibility::Public);
        assert!(
            func.doc_comment
                .as_ref()
                .is_some_and(|d| d.contains("Say hello")),
            "doc: {:?}",
            func.doc_comment
        );
        assert!(
            func.signature.contains("def greet"),
            "sig: {}",
            func.signature
        );
    }

    #[test]
    fn test_parse_async_function() {
        let source = r"
async def fetch_data(url: str) -> bytes:
    pass
";
        let (symbols, _) = parse_py_source(source, "src/net.py").unwrap();
        let func = symbols.iter().find(|s| s.name == "fetch_data").unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
        assert!(
            func.signature.contains("async"),
            "async should be in signature: {}",
            func.signature
        );
    }

    #[test]
    fn test_parse_class_with_methods() {
        let source = r#"
class UserService:
    """Manages users."""

    def __init__(self, db):
        self.db = db

    def get_user(self, user_id: int):
        return self.db.find(user_id)

    def _internal(self):
        pass
"#;
        let (symbols, _) = parse_py_source(source, "src/service.py").unwrap();

        let class = symbols.iter().find(|s| s.name == "UserService").unwrap();
        assert_eq!(class.kind, SymbolKind::Class);
        assert!(
            class
                .doc_comment
                .as_ref()
                .is_some_and(|d| d.contains("Manages users"))
        );

        let get_user = symbols.iter().find(|s| s.name == "get_user").unwrap();
        assert_eq!(get_user.impl_type.as_deref(), Some("UserService"));

        let internal = symbols.iter().find(|s| s.name == "_internal").unwrap();
        assert_eq!(internal.visibility, Visibility::Private);
    }

    #[test]
    fn test_parse_class_inheritance() {
        let source = r"
class Animal:
    pass

class Dog(Animal):
    pass
";
        let (_, trait_impls) = parse_py_source(source, "src/animals.py").unwrap();
        assert_eq!(trait_impls.len(), 1);
        assert_eq!(trait_impls[0].type_name, "Dog");
        assert_eq!(trait_impls[0].trait_name, "Animal");
    }

    #[test]
    fn test_parse_multiple_inheritance() {
        let source = r"
class MyClass(Base, Mixin, Serializable):
    pass
";
        let (_, trait_impls) = parse_py_source(source, "src/multi.py").unwrap();
        assert_eq!(trait_impls.len(), 3);
        let names: Vec<&str> = trait_impls.iter().map(|t| t.trait_name.as_str()).collect();
        assert!(names.contains(&"Base"));
        assert!(names.contains(&"Mixin"));
        assert!(names.contains(&"Serializable"));
    }

    #[test]
    fn test_parse_enum() {
        let source = r#"
from enum import Enum

class Color(Enum):
    RED = "red"
    GREEN = "green"
    BLUE = "blue"
"#;
        let (symbols, trait_impls) = parse_py_source(source, "src/colors.py").unwrap();

        let enum_sym = symbols.iter().find(|s| s.name == "Color").unwrap();
        assert_eq!(enum_sym.kind, SymbolKind::Enum);

        let variants: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::EnumVariant)
            .collect();
        assert_eq!(variants.len(), 3);
        assert!(
            variants
                .iter()
                .all(|v| v.impl_type.as_deref() == Some("Color"))
        );

        assert!(trait_impls.iter().any(|t| t.trait_name == "Enum"));
    }

    #[test]
    fn test_parse_decorated_function() {
        let source = r"
class Foo:
    @property
    def name(self) -> str:
        return self._name

    @staticmethod
    def create():
        return Foo()
";
        let (symbols, _) = parse_py_source(source, "src/foo.py").unwrap();

        let name_prop = symbols.iter().find(|s| s.name == "name").unwrap();
        assert!(
            name_prop
                .attributes
                .as_ref()
                .is_some_and(|a| a.contains("property")),
            "attrs: {:?}",
            name_prop.attributes
        );

        let create = symbols.iter().find(|s| s.name == "create").unwrap();
        assert!(
            create
                .attributes
                .as_ref()
                .is_some_and(|a| a.contains("staticmethod")),
            "attrs: {:?}",
            create.attributes
        );
    }

    #[test]
    fn test_parse_decorated_class() {
        let source = r"
from dataclasses import dataclass

@dataclass
class Point:
    x: int
    y: int
";
        let (symbols, _) = parse_py_source(source, "src/types.py").unwrap();
        let point = symbols.iter().find(|s| s.name == "Point").unwrap();
        assert_eq!(point.kind, SymbolKind::Class);
        assert!(
            point
                .attributes
                .as_ref()
                .is_some_and(|a| a.contains("dataclass")),
            "attrs: {:?}",
            point.attributes
        );
    }

    #[test]
    fn test_parse_imports() {
        let source = r"
import os
from pathlib import Path
from . import utils
";
        let (symbols, _) = parse_py_source(source, "src/main.py").unwrap();
        let imports: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Use)
            .collect();
        assert_eq!(imports.len(), 3);
    }

    #[test]
    fn test_parse_module_constant() {
        let source = r"
MAX_RETRIES = 3
_INTERNAL = 'secret'
";
        let (symbols, _) = parse_py_source(source, "src/config.py").unwrap();

        let max = symbols.iter().find(|s| s.name == "MAX_RETRIES").unwrap();
        assert_eq!(max.kind, SymbolKind::Const);
        assert_eq!(max.visibility, Visibility::Public);

        let internal = symbols.iter().find(|s| s.name == "_INTERNAL").unwrap();
        assert_eq!(internal.visibility, Visibility::Private);
    }

    #[test]
    fn test_visibility_mapping() {
        assert_eq!(get_py_visibility("public"), Visibility::Public);
        assert_eq!(get_py_visibility("_private"), Visibility::Private);
        assert_eq!(get_py_visibility("__mangled"), Visibility::Private);
        assert_eq!(get_py_visibility("__init__"), Visibility::Public);
        assert_eq!(get_py_visibility("__str__"), Visibility::Public);
    }

    #[test]
    fn test_is_test_file() {
        assert!(is_test_py_file("tests/test_utils.py"));
        assert!(is_test_py_file("src/test_service.py"));
        assert!(is_test_py_file("tests/utils_test.py"));
        assert!(is_test_py_file("tests/conftest.py"));
        assert!(!is_test_py_file("src/utils.py"));
        assert!(!is_test_py_file("src/testing.py"));
    }

    #[test]
    fn test_docstring_multiline() {
        let source = r#"
def foo():
    """
    This is a multiline docstring.

    It has multiple paragraphs.
    """
    pass
"#;
        let (symbols, _) = parse_py_source(source, "src/foo.py").unwrap();
        let func = symbols.iter().find(|s| s.name == "foo").unwrap();
        let doc = func.doc_comment.as_ref().unwrap();
        assert!(doc.contains("multiline docstring"), "doc: {doc}");
        assert!(doc.contains("multiple paragraphs"), "doc: {doc}");
    }

    #[test]
    fn test_extract_refs_basic() {
        let source = r"
from service import UserService
from config import Config

def create_app(config: Config):
    service = UserService(config)
    service.run()
";
        let mut known = HashSet::new();
        known.insert("UserService".to_string());
        known.insert("Config".to_string());
        known.insert("create_app".to_string());
        known.insert("run".to_string());

        let tmp = tempfile::TempDir::new().unwrap();
        let refs = extract_py_refs(source, "src/app.py", &known, tmp.path()).unwrap();

        let names: Vec<&str> = refs.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"UserService"), "refs: {names:?}");
        assert!(names.contains(&"Config"), "refs: {names:?}");
    }

    #[test]
    fn test_self_method_ref() {
        let source = r"
class MyService:
    def helper(self):
        pass

    def do_work(self):
        self.helper()
";
        let mut known = HashSet::new();
        known.insert("helper".to_string());
        known.insert("do_work".to_string());

        let tmp = tempfile::TempDir::new().unwrap();
        let refs = extract_py_refs(source, "src/service.py", &known, tmp.path()).unwrap();

        let helper_ref = refs.iter().find(|r| r.target_name == "helper");
        assert!(helper_ref.is_some(), "self.helper() should create a ref");
        assert_eq!(
            helper_ref.unwrap().target_context.as_deref(),
            Some("MyService"),
            "self.method() should resolve to class name"
        );
    }

    #[test]
    fn test_self_not_a_ref() {
        let source = r"
class Foo:
    def bar(self):
        self.baz()
";
        let mut known = HashSet::new();
        known.insert("self".to_string());
        known.insert("bar".to_string());
        known.insert("baz".to_string());

        let tmp = tempfile::TempDir::new().unwrap();
        let refs = extract_py_refs(source, "src/foo.py", &known, tmp.path()).unwrap();

        // "self" should NOT appear as a ref target
        assert!(
            !refs.iter().any(|r| r.target_name == "self"),
            "self should not be a ref"
        );
    }

    #[test]
    fn test_noisy_symbols_filtered() {
        let source = r"
def foo():
    print('hello')
    x = len([1, 2, 3])
    items = list(range(10))
";
        let mut known = HashSet::new();
        known.insert("print".to_string());
        known.insert("len".to_string());
        known.insert("list".to_string());
        known.insert("range".to_string());
        known.insert("foo".to_string());

        let tmp = tempfile::TempDir::new().unwrap();
        let refs = extract_py_refs(source, "src/noisy.py", &known, tmp.path()).unwrap();
        assert!(
            refs.is_empty(),
            "noisy builtins should be filtered: {refs:?}"
        );
    }

    #[test]
    fn test_nested_class() {
        let source = r"
class Outer:
    class Inner:
        def method(self):
            pass
";
        let (symbols, _) = parse_py_source(source, "src/nested.py").unwrap();
        let inner = symbols.iter().find(|s| s.name == "Inner").unwrap();
        assert_eq!(inner.kind, SymbolKind::Class);

        let method = symbols.iter().find(|s| s.name == "method").unwrap();
        assert_eq!(method.impl_type.as_deref(), Some("Inner"));
    }
}
