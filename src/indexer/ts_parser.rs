use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Parser};

use super::parser::{
    extract_body, find_child_by_kind, get_signature, line_range,
    node_text, ImportInfo, RefKind, Symbol, SymbolKind, SymbolRef,
    TraitImpl, Visibility,
};

fn parse_ts(source: &str) -> Result<tree_sitter::Tree, String> {
    std::thread_local! {
        static PARSER: std::cell::RefCell<Option<Parser>> =
            const { std::cell::RefCell::new(None) };
    }
    PARSER.with_borrow_mut(|slot| {
        let parser = slot.get_or_insert_with(|| {
            let mut p = Parser::new();
            let _ = p.set_language(
                &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            );
            p
        });
        parser
            .parse(source, None)
            .ok_or_else(|| "Failed to parse TS source".to_string())
    })
}

fn parse_tsx(source: &str) -> Result<tree_sitter::Tree, String> {
    std::thread_local! {
        static PARSER: std::cell::RefCell<Option<Parser>> =
            const { std::cell::RefCell::new(None) };
    }
    PARSER.with_borrow_mut(|slot| {
        let parser = slot.get_or_insert_with(|| {
            let mut p = Parser::new();
            let _ = p.set_language(
                &tree_sitter_typescript::LANGUAGE_TSX.into(),
            );
            p
        });
        parser
            .parse(source, None)
            .ok_or_else(|| "Failed to parse TSX source".to_string())
    })
}

fn parse_ts_source_tree(
    source: &str,
    file_path: &str,
) -> Result<tree_sitter::Tree, String> {
    if std::path::Path::new(file_path)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tsx"))
    {
        parse_tsx(source)
    } else {
        parse_ts(source)
    }
}

pub fn parse_ts_source(
    source: &str,
    file_path: &str,
) -> Result<(Vec<Symbol>, Vec<TraitImpl>), String> {
    let tree = parse_ts_source_tree(source, file_path)?;
    let root = tree.root_node();
    let mut symbols = Vec::new();
    let mut trait_impls = Vec::new();
    extract_symbols(
        &root,
        source,
        file_path,
        None,
        false,
        &mut symbols,
        &mut trait_impls,
    );
    Ok((symbols, trait_impls))
}

/// Extract `/** ... */` `JSDoc` comment from preceding siblings.
/// For exported declarations, also checks the parent
/// `export_statement`'s preceding siblings since the comment
/// is a sibling of `export_statement`, not `function_declaration`.
fn extract_doc_comment(
    node: &Node,
    source: &str,
) -> Option<String> {
    let mut lines = Vec::new();
    // For exported decls, the prev sibling of the inner node
    // is the `export` keyword, not a comment. Check parent.
    let mut sibling = node.prev_sibling();
    // If prev sibling is the `export` keyword (or absent),
    // look at the export_statement's prev sibling instead
    let should_check_parent = sibling
        .as_ref()
        .is_none_or(|s| s.kind() == "export");
    if should_check_parent
        && let Some(parent) = node.parent()
        && parent.kind() == "export_statement"
    {
        sibling = parent.prev_sibling();
    }
    while let Some(sib) = sibling {
        match sib.kind() {
            "comment" => {
                let text = node_text(&sib, source);
                if let Some(inner) = text.strip_prefix("/**") {
                    let inner =
                        inner.strip_suffix("*/").unwrap_or(inner);
                    // Parse multi-line JSDoc
                    for line in inner.lines() {
                        let trimmed = line.trim();
                        let trimmed = trimmed
                            .strip_prefix("* ")
                            .or_else(|| trimmed.strip_prefix('*'))
                            .unwrap_or(trimmed);
                        if !trimmed.is_empty() {
                            lines.push(trimmed.to_string());
                        }
                    }
                    break;
                } else if text.starts_with("//") {
                    let stripped = text
                        .strip_prefix("///")
                        .or_else(|| text.strip_prefix("//"))
                        .unwrap_or(&text);
                    let stripped =
                        stripped.strip_prefix(' ').unwrap_or(stripped);
                    lines.push(stripped.trim_end().to_string());
                } else {
                    break;
                }
            }
            "decorator" => {
                // Skip decorators between comment and declaration
            }
            _ => break,
        }
        sibling = sib.prev_sibling();
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

/// Extract `@decorator(...)` from preceding siblings.
fn extract_decorators(
    node: &Node,
    source: &str,
) -> Option<String> {
    let mut attrs = Vec::new();
    let mut sibling = node.prev_sibling();
    let should_check_parent = sibling
        .as_ref()
        .is_none_or(|s| s.kind() == "export");
    if should_check_parent
        && let Some(parent) = node.parent()
        && parent.kind() == "export_statement"
    {
        sibling = parent.prev_sibling();
    }
    while let Some(sib) = sibling {
        match sib.kind() {
            "decorator" => {
                let text = node_text(&sib, source);
                let inner =
                    text.strip_prefix('@').unwrap_or(&text);
                attrs.push(inner.trim().to_string());
            }
            "comment" => {}
            _ => break,
        }
        sibling = sib.prev_sibling();
    }
    if attrs.is_empty() {
        return None;
    }
    attrs.reverse();
    Some(attrs.join(", "))
}

/// Whether a node has an `export` keyword as a parent or itself.
fn is_exported(node: &Node) -> bool {
    if let Some(parent) = node.parent() {
        parent.kind() == "export_statement"
    } else {
        false
    }
}

fn get_visibility(node: &Node) -> Visibility {
    if is_exported(node) {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

/// Get visibility for a class member based on accessibility modifier.
fn get_member_visibility(node: &Node, source: &str) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "accessibility_modifier" {
            let text = node_text(&child, source);
            return match text.as_str() {
                "private" => Visibility::Private,
                "protected" => Visibility::Restricted,
                _ => Visibility::Public,
            };
        }
    }
    // Default class member visibility is public in TS
    Visibility::Public
}

fn extract_symbols(
    node: &Node,
    source: &str,
    file_path: &str,
    class_name: Option<&str>,
    exported: bool,
    symbols: &mut Vec<Symbol>,
    trait_impls: &mut Vec<TraitImpl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration"
            | "generator_function_declaration" => {
                if let Some(mut sym) =
                    extract_function(&child, source, file_path)
                {
                    if exported {
                        sym.visibility = Visibility::Public;
                    }
                    sym.impl_type =
                        class_name.map(String::from);
                    symbols.push(sym);
                }
            }
            "class_declaration"
            | "abstract_class_declaration" => {
                extract_class(
                    &child, source, file_path, exported,
                    symbols, trait_impls,
                );
            }
            "interface_declaration" => {
                extract_interface(
                    &child, source, file_path, exported,
                    symbols, trait_impls,
                );
            }
            "type_alias_declaration" => {
                if let Some(mut sym) = extract_type_alias(
                    &child, source, file_path,
                ) {
                    if exported {
                        sym.visibility = Visibility::Public;
                    }
                    symbols.push(sym);
                }
            }
            "enum_declaration" => {
                extract_enum(
                    &child, source, file_path, exported,
                    symbols,
                );
            }
            "lexical_declaration"
            | "variable_declaration" => {
                extract_variable_declarations(
                    &child, source, file_path, exported, symbols,
                );
            }
            "export_statement" => {
                extract_symbols(
                    &child, source, file_path, class_name,
                    true, symbols, trait_impls,
                );
            }
            "import_statement" => {
                extract_import(
                    &child, source, file_path, symbols,
                );
            }
            "method_definition"
            | "public_field_definition"
            | "abstract_method_definition" => {
                if class_name.is_some() && let Some(sym) = extract_class_member(
                    &child, source, file_path, class_name,
                ) {
                    symbols.push(sym);
                }
            }
            "module" | "internal_module" => {
                extract_module(
                    &child, source, file_path, exported,
                    symbols, trait_impls,
                );
            }
            "statement_block" if class_name.is_none() => {
                extract_symbols(
                    &child, source, file_path, None, false,
                    symbols, trait_impls,
                );
            }
            _ => {}
        }
    }
}

fn extract_module(
    child: &Node,
    source: &str,
    file_path: &str,
    exported: bool,
    symbols: &mut Vec<Symbol>,
    trait_impls: &mut Vec<TraitImpl>,
) {
    if let Some(name_node) =
        find_child_by_kind(child, "identifier")
    {
        let name = node_text(&name_node, source);
        let (line_start, line_end) = line_range(child);
        let mut vis = get_visibility(child);
        if exported {
            vis = Visibility::Public;
        }
        symbols.push(Symbol {
            name,
            kind: SymbolKind::Mod,
            visibility: vis,
            file_path: file_path.to_string(),
            line_start,
            line_end,
            signature: get_signature(child, source),
            doc_comment: extract_doc_comment(child, source),
            body: Some(extract_body(child, source)),
            details: None,
            attributes: None,
            impl_type: None,
        });
    }
    if let Some(body) =
        find_child_by_kind(child, "statement_block")
    {
        extract_symbols(
            &body, source, file_path, None, exported,
            symbols, trait_impls,
        );
    }
}

fn extract_function(
    node: &Node,
    source: &str,
    file_path: &str,
) -> Option<Symbol> {
    let name_node = find_child_by_kind(node, "identifier")?;
    let name = node_text(&name_node, source);
    let (line_start, line_end) = line_range(node);

    Some(Symbol {
        name,
        kind: SymbolKind::Function,
        visibility: get_visibility(node),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: get_signature(node, source),
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details: None,
        attributes: extract_decorators(node, source),
        impl_type: None,
    })
}

fn extract_class(
    node: &Node,
    source: &str,
    file_path: &str,
    exported: bool,
    symbols: &mut Vec<Symbol>,
    trait_impls: &mut Vec<TraitImpl>,
) {
    let Some(name_node) =
        find_child_by_kind(node, "type_identifier")
    else {
        return;
    };
    let class_name = node_text(&name_node, source);
    let (line_start, line_end) = line_range(node);
    let mut vis = get_visibility(node);
    if exported {
        vis = Visibility::Public;
    }

    // Extract heritage: extends and implements
    let mut details_parts = Vec::new();
    extract_heritage(
        node,
        source,
        &HeritageContext {
            file_path,
            class_name: &class_name,
            line_start,
            line_end,
        },
        &mut details_parts,
        trait_impls,
    );

    symbols.push(Symbol {
        name: class_name.clone(),
        kind: SymbolKind::Class,
        visibility: vis,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: get_signature(node, source),
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details: if details_parts.is_empty() {
            None
        } else {
            Some(details_parts.join("\n"))
        },
        attributes: extract_decorators(node, source),
        impl_type: None,
    });

    // Recurse into class body for methods/properties
    if let Some(body) = find_child_by_kind(node, "class_body") {
        extract_symbols(
            &body,
            source,
            file_path,
            Some(&class_name),
            false,
            symbols,
            trait_impls,
        );
    }
}

struct HeritageContext<'a> {
    file_path: &'a str,
    class_name: &'a str,
    line_start: usize,
    line_end: usize,
}

fn extract_heritage(
    node: &Node,
    source: &str,
    hctx: &HeritageContext<'_>,
    details: &mut Vec<String>,
    trait_impls: &mut Vec<TraitImpl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_heritage" {
            let mut inner = child.walk();
            for heritage in child.children(&mut inner) {
                let keyword = match heritage.kind() {
                    "extends_clause" => "extends",
                    "implements_clause" => "implements",
                    _ => continue,
                };
                let names =
                    collect_type_names(&heritage, source);
                for name in &names {
                    details.push(format!(
                        "{keyword} {name}"
                    ));
                    trait_impls.push(TraitImpl {
                        type_name: hctx.class_name
                            .to_string(),
                        trait_name: name.clone(),
                        file_path: hctx.file_path
                            .to_string(),
                        line_start: hctx.line_start,
                        line_end: hctx.line_end,
                    });
                }
            }
        }
    }
}

/// Collect type names from extends/implements clauses.
fn collect_type_names(
    clause: &Node,
    source: &str,
) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = clause.walk();
    for child in clause.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                names.push(node_text(&child, source));
            }
            "generic_type" => {
                // Extract the base type from Generic<T>
                if let Some(base) =
                    find_child_by_kind(&child, "type_identifier")
                {
                    names.push(node_text(&base, source));
                }
            }
            "member_expression" | "nested_type_identifier" => {
                // Foo.Bar or Foo.Bar.Baz — take last segment
                let text = node_text(&child, source);
                if let Some(last) = text.rsplit('.').next() {
                    names.push(last.to_string());
                }
            }
            _ => {}
        }
    }
    names
}

fn extract_interface(
    node: &Node,
    source: &str,
    file_path: &str,
    exported: bool,
    symbols: &mut Vec<Symbol>,
    trait_impls: &mut Vec<TraitImpl>,
) {
    let Some(name_node) =
        find_child_by_kind(node, "type_identifier")
    else {
        return;
    };
    let name = node_text(&name_node, source);
    let (line_start, line_end) = line_range(node);
    let mut vis = get_visibility(node);
    if exported {
        vis = Visibility::Public;
    }

    // Extract interface body as details
    let details =
        extract_interface_details(node, source);

    symbols.push(Symbol {
        name: name.clone(),
        kind: SymbolKind::Interface,
        visibility: vis,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: get_signature(node, source),
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details,
        attributes: None,
        impl_type: None,
    });

    // Check for interface extends
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "extends_type_clause" {
            let names = collect_type_names(&child, source);
            for type_name in &names {
                trait_impls.push(TraitImpl {
                    type_name: name.clone(),
                    trait_name: type_name.clone(),
                    file_path: file_path.to_string(),
                    line_start,
                    line_end,
                });
            }
        }
    }
}

fn extract_interface_details(
    node: &Node,
    source: &str,
) -> Option<String> {
    let body = find_child_by_kind(node, "object_type")
        .or_else(|| {
            find_child_by_kind(node, "interface_body")
        })?;
    let mut members = Vec::new();
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "property_signature" | "method_signature"
            | "call_signature" | "construct_signature"
            | "index_signature" => {
                let text = node_text(&child, source)
                    .trim_end()
                    .to_string();
                let text = text
                    .strip_suffix(';')
                    .or_else(|| text.strip_suffix(','))
                    .unwrap_or(&text)
                    .to_string();
                members.push(text);
            }
            _ => {}
        }
    }
    if members.is_empty() {
        return None;
    }
    Some(members.join("\n"))
}

fn extract_type_alias(
    node: &Node,
    source: &str,
    file_path: &str,
) -> Option<Symbol> {
    let name_node =
        find_child_by_kind(node, "type_identifier")?;
    let name = node_text(&name_node, source);
    let (line_start, line_end) = line_range(node);

    Some(Symbol {
        name,
        kind: SymbolKind::TypeAlias,
        visibility: get_visibility(node),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: get_signature(node, source),
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details: None,
        attributes: None,
        impl_type: None,
    })
}

fn extract_enum(
    node: &Node,
    source: &str,
    file_path: &str,
    exported: bool,
    symbols: &mut Vec<Symbol>,
) {
    let Some(name_node) =
        find_child_by_kind(node, "identifier")
    else {
        return;
    };
    let enum_name = node_text(&name_node, source);
    let (line_start, line_end) = line_range(node);
    let mut vis = get_visibility(node);
    if exported {
        vis = Visibility::Public;
    }

    // Extract variants as details
    let details = extract_enum_body_details(node, source);

    symbols.push(Symbol {
        name: enum_name.clone(),
        kind: SymbolKind::Enum,
        visibility: vis.clone(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: get_signature(node, source),
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details,
        attributes: None,
        impl_type: None,
    });

    // Extract individual enum members as EnumVariant
    if let Some(body) = find_child_by_kind(node, "enum_body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "enum_assignment"
                || child.kind() == "property_identifier"
            {
                extract_enum_member(
                    &child,
                    source,
                    file_path,
                    &enum_name,
                    &vis,
                    symbols,
                );
            }
        }
    }
}

fn extract_enum_member(
    node: &Node,
    source: &str,
    file_path: &str,
    enum_name: &str,
    vis: &Visibility,
    symbols: &mut Vec<Symbol>,
) {
    let name = if node.kind() == "property_identifier" {
        node_text(node, source)
    } else {
        // enum_assignment: member = value
        find_child_by_kind(node, "property_identifier")
            .or_else(|| {
                find_child_by_kind(node, "identifier")
            })
            .map_or_else(String::new, |n| node_text(&n, source))
    };
    if name.is_empty() {
        return;
    }
    let (line_start, line_end) = line_range(node);
    let text = node_text(node, source).trim_end().to_string();
    let text = text
        .strip_suffix(',')
        .unwrap_or(&text)
        .to_string();

    symbols.push(Symbol {
        name,
        kind: SymbolKind::EnumVariant,
        visibility: vis.clone(),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: text,
        doc_comment: None,
        body: None,
        details: None,
        attributes: None,
        impl_type: Some(enum_name.to_string()),
    });
}

fn extract_enum_body_details(
    node: &Node,
    source: &str,
) -> Option<String> {
    let body = find_child_by_kind(node, "enum_body")?;
    let mut variants = Vec::new();
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "enum_assignment" | "property_identifier" => {
                let text = node_text(&child, source)
                    .trim_end()
                    .to_string();
                let text = text
                    .strip_suffix(',')
                    .unwrap_or(&text)
                    .to_string();
                variants.push(text);
            }
            _ => {}
        }
    }
    if variants.is_empty() {
        return None;
    }
    Some(variants.join("\n"))
}

fn extract_variable_declarations(
    node: &Node,
    source: &str,
    file_path: &str,
    exported: bool,
    symbols: &mut Vec<Symbol>,
) {
    // `const foo = 1` or `const foo: Type = ...`
    let is_const = node_text(node, source).starts_with("const");
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name_node =
                find_child_by_kind(&child, "identifier")
                    .or_else(|| {
                        find_child_by_kind(
                            &child,
                            "type_identifier",
                        )
                    });
            let Some(name_node) = name_node else {
                continue;
            };
            let name = node_text(&name_node, source);
            let (line_start, line_end) = line_range(node);
            let mut vis = get_visibility(node);
            if exported {
                vis = Visibility::Public;
            }

            // Check if the value is an arrow function
            let value_node =
                child.child_by_field_name("value");
            let is_arrow = value_node.as_ref().is_some_and(
                |v| v.kind() == "arrow_function",
            );

            let kind = if is_arrow {
                SymbolKind::Function
            } else if is_const {
                SymbolKind::Const
            } else {
                SymbolKind::Static
            };

            symbols.push(Symbol {
                name,
                kind,
                visibility: vis,
                file_path: file_path.to_string(),
                line_start,
                line_end,
                signature: get_signature(node, source),
                doc_comment: extract_doc_comment(
                    node, source,
                ),
                body: if is_arrow {
                    value_node
                        .map(|v| extract_body(&v, source))
                } else {
                    None
                },
                details: None,
                attributes: extract_decorators(node, source),
                impl_type: None,
            });
        }
    }
}

fn extract_class_member(
    node: &Node,
    source: &str,
    file_path: &str,
    class_name: Option<&str>,
) -> Option<Symbol> {
    let name_node =
        find_child_by_kind(node, "property_identifier")
            .or_else(|| {
                find_child_by_kind(node, "identifier")
            })?;
    let name = node_text(&name_node, source);
    let (line_start, line_end) = line_range(node);

    let kind = if node.kind() == "method_definition"
        || node.kind() == "abstract_method_definition"
    {
        SymbolKind::Function
    } else {
        // public_field_definition
        SymbolKind::Const
    };

    Some(Symbol {
        name,
        kind,
        visibility: get_member_visibility(node, source),
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: get_signature(node, source),
        doc_comment: extract_doc_comment(node, source),
        body: if kind == SymbolKind::Function {
            Some(extract_body(node, source))
        } else {
            None
        },
        details: None,
        attributes: extract_decorators(node, source),
        impl_type: class_name.map(String::from),
    })
}

fn extract_import(
    node: &Node,
    source: &str,
    file_path: &str,
    symbols: &mut Vec<Symbol>,
) {
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

/// Check if a file path indicates a test file.
#[must_use]
pub fn is_test_ts_file(path: &str) -> bool {
    let p = std::path::Path::new(path);
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let ext = p
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    // Check for double extensions: .test.ts, .spec.ts, etc
    let stem_ext = std::path::Path::new(stem)
        .extension()
        .and_then(|e| e.to_str());
    if matches!(stem_ext, Some("test" | "spec")) {
        return matches!(ext, "ts" | "tsx" | "js" | "jsx");
    }

    // Check for __tests__ directory
    path.contains("__tests__/")
        || path.contains("__tests__\\")
}

/// Noisy symbols for TypeScript — common built-in methods
/// that would create false positive refs.
const TS_NOISY_SYMBOLS: &[&str] = &[
    // Built-in constructors/types (always available)
    "Array",
    "Boolean",
    "Date",
    "Error",
    "Map",
    "Number",
    "Object",
    "Promise",
    "RegExp",
    "Set",
    "String",
    "Symbol",
    // Common methods
    "addEventListener",
    "apply",
    "bind",
    "call",
    "catch",
    "concat",
    "entries",
    "every",
    "filter",
    "find",
    "findIndex",
    "flat",
    "flatMap",
    "forEach",
    "from",
    "get",
    "has",
    "includes",
    "indexOf",
    "join",
    "keys",
    "length",
    "log",
    "map",
    "of",
    "pop",
    "push",
    "reduce",
    "replace",
    "set",
    "shift",
    "slice",
    "some",
    "sort",
    "splice",
    "split",
    "startsWith",
    "then",
    "toString",
    "trim",
    "unshift",
    "values",
    // Console and common globals
    "console",
    "document",
    "fetch",
    "require",
    "setTimeout",
    "setInterval",
    "window",
];

fn is_noisy_ts_symbol(name: &str) -> bool {
    TS_NOISY_SYMBOLS.contains(&name)
}

struct TsRefContext<'a, S: std::hash::BuildHasher> {
    source: &'a str,
    file_path: &'a str,
    known_symbols: &'a HashSet<String, S>,
    import_map: HashMap<String, ImportInfo>,
}

/// Extract references from TS source code.
pub fn extract_ts_refs<S: std::hash::BuildHasher>(
    source: &str,
    file_path: &str,
    known_symbols: &HashSet<String, S>,
) -> Result<Vec<SymbolRef>, String> {
    let tree = parse_ts_source_tree(source, file_path)?;
    let root = tree.root_node();
    let import_map = build_ts_import_map(&root, source);
    let ctx = TsRefContext {
        source,
        file_path,
        known_symbols,
        import_map,
    };
    let mut refs = Vec::new();
    collect_ts_refs(&root, &ctx, None, &mut refs);
    Ok(refs)
}

/// Build import map from `import` declarations.
fn build_ts_import_map(
    root: &Node,
    source: &str,
) -> HashMap<String, ImportInfo> {
    let mut map = HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "import_statement" {
            parse_import_statement(&child, source, &mut map);
        }
    }
    map
}

fn parse_import_statement(
    node: &Node,
    source: &str,
    map: &mut HashMap<String, ImportInfo>,
) {
    // Extract the source module path
    let source_path = find_child_by_kind(node, "string")
        .map(|n| {
            let text = node_text(&n, source);
            text.trim_matches(|c| c == '\'' || c == '"')
                .to_string()
        });
    let Some(module_path) = source_path else {
        return;
    };

    // Extract imported names
    let mut inner = node.walk();
    for child in node.children(&mut inner) {
        if child.kind() == "import_clause" {
            parse_import_clause(
                &child,
                source,
                &module_path,
                map,
            );
        }
    }
}

fn parse_import_clause(
    node: &Node,
    source: &str,
    module_path: &str,
    map: &mut HashMap<String, ImportInfo>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Default import: import Foo from '...'
            "identifier" => {
                let name = node_text(&child, source);
                map.insert(
                    name,
                    ImportInfo {
                        qualified_path: module_path
                            .to_string(),
                    },
                );
            }
            // Named imports: import { Foo, Bar as Baz } from '...'
            "named_imports" => {
                let mut inner = child.walk();
                for spec in child.children(&mut inner) {
                    if spec.kind() == "import_specifier" {
                        // Check for alias: import { Foo as Bar }
                        let alias = spec
                            .child_by_field_name("alias")
                            .map(|n| node_text(&n, source));
                        let name = find_child_by_kind(
                            &spec,
                            "identifier",
                        )
                        .map(|n| node_text(&n, source));
                        let local_name =
                            alias.or(name);
                        if let Some(local) = local_name {
                            map.insert(
                                local,
                                ImportInfo {
                                    qualified_path:
                                        module_path
                                            .to_string(),
                                },
                            );
                        }
                    }
                }
            }
            // Namespace import: import * as Foo from '...'
            "namespace_import" => {
                if let Some(name_node) =
                    find_child_by_kind(&child, "identifier")
                {
                    let name =
                        node_text(&name_node, source);
                    map.insert(
                        name,
                        ImportInfo {
                            qualified_path: module_path
                                .to_string(),
                        },
                    );
                }
            }
            _ => {}
        }
    }
}

fn collect_ts_refs<S: std::hash::BuildHasher>(
    node: &Node,
    ctx: &TsRefContext<'_, S>,
    class_name: Option<&str>,
    refs: &mut Vec<SymbolRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration"
            | "generator_function_declaration" => {
                let Some(name_node) =
                    find_child_by_kind(&child, "identifier")
                else {
                    continue;
                };
                let fn_name =
                    node_text(&name_node, ctx.source);
                collect_ts_body_refs(
                    &child, &fn_name, class_name, ctx, refs,
                );
            }
            "class_declaration"
            | "abstract_class_declaration" => {
                let type_name =
                    find_child_by_kind(&child, "type_identifier")
                        .map(|n| node_text(&n, ctx.source));
                if let Some(body) =
                    find_child_by_kind(&child, "class_body")
                {
                    collect_ts_refs(
                        &body,
                        ctx,
                        type_name.as_deref(),
                        refs,
                    );
                }
            }
            "method_definition"
            | "abstract_method_definition" => {
                let name_node = find_child_by_kind(
                    &child,
                    "property_identifier",
                )
                .or_else(|| {
                    find_child_by_kind(&child, "identifier")
                });
                let Some(name_node) = name_node else {
                    continue;
                };
                let method_name =
                    node_text(&name_node, ctx.source);
                collect_ts_body_refs(
                    &child,
                    &method_name,
                    class_name,
                    ctx,
                    refs,
                );
            }
            "lexical_declaration" => {
                // const foo = (...) => { ... }
                let mut inner = child.walk();
                for decl in child.children(&mut inner) {
                    if decl.kind() == "variable_declarator" {
                        let name = find_child_by_kind(
                            &decl,
                            "identifier",
                        )
                        .map(|n| node_text(&n, ctx.source));
                        let value =
                            decl.child_by_field_name("value");
                        if let (Some(name), Some(value)) =
                            (name, value)
                            && (value.kind() == "arrow_function"
                                || value.kind() == "function"
                                || value.kind() == "function_expression")
                        {
                            collect_ts_body_refs(
                                &value,
                                &name,
                                class_name,
                                ctx,
                                refs,
                            );
                        }
                    }
                }
            }
            "export_statement" => {
                collect_ts_refs(
                    &child, ctx, class_name, refs,
                );
            }
            "module" | "internal_module" => {
                if let Some(body) = find_child_by_kind(
                    &child,
                    "statement_block",
                ) {
                    collect_ts_refs(
                        &body, ctx, None, refs,
                    );
                }
            }
            _ => {}
        }
    }
}

fn collect_ts_locals(
    node: &Node,
    source: &str,
) -> HashSet<String> {
    let mut locals = HashSet::new();
    let mut stack = vec![*node];
    while let Some(n) = stack.pop() {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            match child.kind() {
                "required_parameter"
                | "optional_parameter"
                | "variable_declarator" => {
                    if let Some(id) =
                        find_child_by_kind(&child, "identifier")
                    {
                        locals.insert(node_text(&id, source));
                    }
                }
                _ => stack.push(child),
            }
        }
    }
    locals
}

fn collect_ts_body_refs<S: std::hash::BuildHasher>(
    node: &Node,
    fn_name: &str,
    impl_type: Option<&str>,
    ctx: &TsRefContext<'_, S>,
    refs: &mut Vec<SymbolRef>,
) {
    let locals = collect_ts_locals(node, ctx.source);
    let mut seen = HashSet::new();
    let mut stack = vec![*node];

    while let Some(n) = stack.pop() {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            match child.kind() {
                // Function call: foo(...)
                "call_expression" => {
                    if let Some(func) = child.child(0) {
                        process_ts_call(
                            &func, fn_name, impl_type, ctx,
                            &locals, &mut seen, refs,
                        );
                    }
                    stack.push(child);
                }
                // new Foo(...)
                "new_expression" => {
                    if let Some(constructor) = child.child(1) {
                        let name =
                            node_text(&constructor, ctx.source);
                        try_add_ts_ref(
                            &name,
                            &TsRefInfo {
                                source_name: fn_name,
                                target_context: None,
                                kind: RefKind::Call,
                                ref_line: i64::try_from(
                                    child.start_position().row,
                                )
                                .unwrap_or(0)
                                    + 1,
                            },
                            ctx,
                            &locals,
                            &mut seen,
                            refs,
                        );
                    }
                    stack.push(child);
                }
                // Type references in annotations
                "type_identifier" => {
                    let name =
                        node_text(&child, ctx.source);
                    try_add_ts_ref(
                        &name,
                        &TsRefInfo {
                            source_name: fn_name,
                            target_context: None,
                            kind: RefKind::TypeRef,
                            ref_line: i64::try_from(
                                child.start_position().row,
                            )
                            .unwrap_or(0)
                                + 1,
                        },
                        ctx,
                        &locals,
                        &mut seen,
                        refs,
                    );
                }
                // Skip nested function/class defs
                "function_declaration"
                | "class_declaration"
                | "arrow_function"
                    if n.kind() != node.kind() =>
                {
                    // Don't descend into nested definitions
                }
                _ => {
                    stack.push(child);
                }
            }
        }
    }
}

fn process_ts_call<S: std::hash::BuildHasher>(
    func: &Node,
    fn_name: &str,
    impl_type: Option<&str>,
    ctx: &TsRefContext<'_, S>,
    locals: &HashSet<String>,
    seen: &mut HashSet<String>,
    refs: &mut Vec<SymbolRef>,
) {
    let ref_line =
        i64::try_from(func.start_position().row)
            .unwrap_or(0)
            + 1;
    match func.kind() {
        "identifier" => {
            let name = node_text(func, ctx.source);
            try_add_ts_ref(
                &name,
                &TsRefInfo {
                    source_name: fn_name,
                    target_context: None,
                    kind: RefKind::Call,
                    ref_line,
                },
                ctx, locals, seen, refs,
            );
        }
        "member_expression" => {
            // obj.method() — extract method name and
            // try to resolve obj type
            if let Some(prop) = find_child_by_kind(
                func,
                "property_identifier",
            ) {
                let method_name =
                    node_text(&prop, ctx.source);
                let obj = func.child(0);
                let target_context = obj.and_then(|o| {
                    match o.kind() {
                        // this.method() → use class name
                        "this" => {
                            impl_type.map(String::from)
                        }
                        "identifier" => {
                            let name =
                                node_text(&o, ctx.source);
                            // Uppercase = likely type
                            if name
                                .chars()
                                .next()
                                .is_some_and(
                                    char::is_uppercase,
                                )
                            {
                                Some(name)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                });
                try_add_ts_ref(
                    &method_name,
                    &TsRefInfo {
                        source_name: fn_name,
                        target_context: target_context
                            .as_deref(),
                        kind: RefKind::Call,
                        ref_line,
                    },
                    ctx,
                    locals,
                    seen,
                    refs,
                );
            }
        }
        _ => {}
    }
}

struct TsRefInfo<'a> {
    source_name: &'a str,
    target_context: Option<&'a str>,
    kind: RefKind,
    ref_line: i64,
}

fn try_add_ts_ref<S: std::hash::BuildHasher>(
    name: &str,
    info: &TsRefInfo<'_>,
    ctx: &TsRefContext<'_, S>,
    locals: &HashSet<String>,
    seen: &mut HashSet<String>,
    refs: &mut Vec<SymbolRef>,
) {
    if name.is_empty() || name == info.source_name {
        return;
    }

    // Dedup key includes context
    let dedup_key =
        if let Some(tc) = info.target_context {
            format!("{tc}::{name}")
        } else {
            name.to_string()
        };
    if !seen.insert(dedup_key) {
        return;
    }

    // Skip local variables
    if locals.contains(name) {
        return;
    }

    // Skip noisy symbols unless qualified
    if info.target_context.is_none()
        && is_noisy_ts_symbol(name)
    {
        return;
    }

    // Check if target is a known symbol
    if !ctx.known_symbols.contains(name) {
        return;
    }

    // Import-resolved refs get target_file set, which DB layer
    // uses for high-confidence matching
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

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_parse_function() {
        let source = r"
export function greet(name: string): string {
    return `Hello, ${name}!`;
}
";
        let (symbols, _) =
            parse_ts_source(source, "src/lib.ts").unwrap();
        let func = symbols
            .iter()
            .find(|s| s.name == "greet")
            .unwrap();
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.visibility, Visibility::Public);
    }

    #[test]
    fn test_parse_class_with_methods() {
        let source = r"
export class UserService {
    private name: string;

    constructor(name: string) {
        this.name = name;
    }

    getName(): string {
        return this.name;
    }
}
";
        let (symbols, _) =
            parse_ts_source(source, "src/service.ts").unwrap();

        let class = symbols
            .iter()
            .find(|s| s.name == "UserService")
            .unwrap();
        assert_eq!(class.kind, SymbolKind::Class);
        assert_eq!(class.visibility, Visibility::Public);

        let method = symbols
            .iter()
            .find(|s| {
                s.name == "getName"
                    && s.kind == SymbolKind::Function
            })
            .unwrap();
        assert_eq!(
            method.impl_type.as_deref(),
            Some("UserService")
        );
    }

    #[test]
    fn test_parse_interface() {
        let source = r"
export interface Serializable {
    serialize(): string;
    deserialize(data: string): void;
}
";
        let (symbols, _) =
            parse_ts_source(source, "src/types.ts").unwrap();
        let iface = symbols
            .iter()
            .find(|s| s.name == "Serializable")
            .unwrap();
        assert_eq!(iface.kind, SymbolKind::Interface);
        assert_eq!(iface.visibility, Visibility::Public);
        assert!(iface.details.is_some());
    }

    #[test]
    fn test_parse_class_implements() {
        let source = r"
interface Runnable {
    run(): void;
}

class Task implements Runnable {
    run(): void {}
}
";
        let (_, trait_impls) =
            parse_ts_source(source, "src/task.ts").unwrap();
        assert_eq!(trait_impls.len(), 1);
        assert_eq!(trait_impls[0].type_name, "Task");
        assert_eq!(trait_impls[0].trait_name, "Runnable");
    }

    #[test]
    fn test_parse_class_extends() {
        let source = r"
class Animal {
    name: string;
}

class Dog extends Animal {
    breed: string;
}
";
        let (_, trait_impls) =
            parse_ts_source(source, "src/animals.ts").unwrap();
        assert_eq!(trait_impls.len(), 1);
        assert_eq!(trait_impls[0].type_name, "Dog");
        assert_eq!(trait_impls[0].trait_name, "Animal");
    }

    #[test]
    fn test_parse_enum() {
        let source = r#"
export enum Color {
    Red = "RED",
    Green = "GREEN",
    Blue = "BLUE",
}
"#;
        let (symbols, _) =
            parse_ts_source(source, "src/color.ts").unwrap();
        let enum_sym = symbols
            .iter()
            .find(|s| {
                s.name == "Color"
                    && s.kind == SymbolKind::Enum
            })
            .unwrap();
        assert_eq!(enum_sym.visibility, Visibility::Public);

        let variants: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::EnumVariant)
            .collect();
        assert_eq!(variants.len(), 3);
        assert!(variants
            .iter()
            .all(|v| v.impl_type.as_deref()
                == Some("Color")));
    }

    #[test]
    fn test_parse_type_alias() {
        let source = r"
export type Result<T> = { ok: true; value: T } | { ok: false; error: Error };
";
        let (symbols, _) =
            parse_ts_source(source, "src/types.ts").unwrap();
        let ta = symbols
            .iter()
            .find(|s| s.name == "Result")
            .unwrap();
        assert_eq!(ta.kind, SymbolKind::TypeAlias);
    }

    #[test]
    fn test_parse_const_export() {
        let source = r"
export const MAX_RETRIES = 3;
";
        let (symbols, _) =
            parse_ts_source(source, "src/config.ts").unwrap();
        let c = symbols
            .iter()
            .find(|s| s.name == "MAX_RETRIES")
            .unwrap();
        assert_eq!(c.kind, SymbolKind::Const);
        assert_eq!(c.visibility, Visibility::Public);
    }

    #[test]
    fn test_parse_arrow_function() {
        let source = r"
export const add = (a: number, b: number): number => a + b;
";
        let (symbols, _) =
            parse_ts_source(source, "src/math.ts").unwrap();
        let f = symbols
            .iter()
            .find(|s| s.name == "add")
            .unwrap();
        assert_eq!(f.kind, SymbolKind::Function);
        assert_eq!(f.visibility, Visibility::Public);
    }

    #[test]
    fn test_parse_import() {
        let source = r"
import { Foo, Bar } from './foo';
import type { Baz } from './baz';
";
        let (symbols, _) =
            parse_ts_source(source, "src/main.ts").unwrap();
        let imports: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Use)
            .collect();
        assert_eq!(imports.len(), 2);
    }

    #[test]
    fn test_is_test_file() {
        assert!(is_test_ts_file("src/utils.test.ts"));
        assert!(is_test_ts_file("src/utils.spec.ts"));
        assert!(is_test_ts_file(
            "src/__tests__/utils.ts"
        ));
        assert!(is_test_ts_file("src/App.test.tsx"));
        assert!(!is_test_ts_file("src/utils.ts"));
        assert!(!is_test_ts_file("src/test-utils.ts"));
    }

    #[test]
    fn test_extract_refs() {
        let source = r"
import { UserService } from './service';
import { Config } from './config';

export function createApp(config: Config): void {
    const service = new UserService(config);
    service.getName();
}
";
        let mut known = HashSet::new();
        known.insert("UserService".to_string());
        known.insert("Config".to_string());
        known.insert("getName".to_string());
        known.insert("createApp".to_string());

        let refs =
            extract_ts_refs(source, "src/app.ts", &known)
                .unwrap();

        // Should find refs to UserService, Config, getName
        let names: Vec<&str> = refs
            .iter()
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            names.contains(&"UserService"),
            "refs: {names:?}"
        );
        assert!(
            names.contains(&"Config"),
            "refs: {names:?}"
        );
    }

    #[test]
    fn test_jsdoc_extraction() {
        let source = r"
/**
 * Adds two numbers together.
 * @param a First number
 * @param b Second number
 * @returns The sum
 */
export function add(a: number, b: number): number {
    return a + b;
}
";
        let (symbols, _) =
            parse_ts_source(source, "src/math.ts").unwrap();
let f = symbols
            .iter()
            .find(|s| s.name == "add")
            .unwrap();
        let doc = f.doc_comment.as_ref().unwrap();
        assert!(doc.contains("Adds two numbers"));
        assert!(doc.contains("@param a"));
    }

    #[test]
    fn test_tsx_parsing() {
        let source = r"
import React from 'react';

interface Props {
    name: string;
}

export function Greeting({ name }: Props): JSX.Element {
    return <div>Hello, {name}!</div>;
}
";
        let (symbols, _) =
            parse_ts_source(source, "src/Greeting.tsx")
                .unwrap();
        let component = symbols
            .iter()
            .find(|s| s.name == "Greeting")
            .unwrap();
        assert_eq!(component.kind, SymbolKind::Function);
        assert_eq!(
            component.visibility,
            Visibility::Public
        );

        let iface = symbols
            .iter()
            .find(|s| s.name == "Props")
            .unwrap();
        assert_eq!(iface.kind, SymbolKind::Interface);
    }

    #[test]
    fn test_private_class_members() {
        let source = r"
class Foo {
    private secret: string;
    protected bar(): void {}
    public baz(): void {}
    qux(): void {}
}
";
        let (symbols, _) =
            parse_ts_source(source, "src/foo.ts").unwrap();

        let bar = symbols
            .iter()
            .find(|s| s.name == "bar")
            .unwrap();
        assert_eq!(
            bar.visibility,
            Visibility::Restricted,
            "protected should be Restricted"
        );

        let secret = symbols
            .iter()
            .find(|s| s.name == "secret")
            .unwrap();
        assert_eq!(secret.visibility, Visibility::Private);

        let baz = symbols
            .iter()
            .find(|s| s.name == "baz")
            .unwrap();
        assert_eq!(baz.visibility, Visibility::Public);

        let qux = symbols
            .iter()
            .find(|s| s.name == "qux")
            .unwrap();
        assert_eq!(
            qux.visibility,
            Visibility::Public,
            "default class member visibility is public"
        );
    }

    #[test]
    fn test_interface_extends() {
        let source = r"
interface Base {
    id: number;
}

interface Extended extends Base {
    name: string;
}
";
        let (symbols, trait_impls) =
            parse_ts_source(source, "src/types.ts").unwrap();
        assert!(symbols.iter().any(|s| s.name == "Extended"
            && s.kind == SymbolKind::Interface));
        assert_eq!(trait_impls.len(), 1);
        assert_eq!(trait_impls[0].type_name, "Extended");
        assert_eq!(trait_impls[0].trait_name, "Base");
    }

    #[test]
    fn test_var_declaration() {
        let source = r"
export var legacyConfig = { debug: true };
";
        let (symbols, _) =
            parse_ts_source(source, "src/legacy.ts").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "legacyConfig")
            .unwrap();
        assert_eq!(sym.kind, SymbolKind::Static);
    }
}
