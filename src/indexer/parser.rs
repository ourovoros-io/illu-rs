use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    EnumVariant,
    Trait,
    Impl,
    Use,
    Mod,
    Const,
    Static,
    TypeAlias,
    Macro,
    Union,
    Interface,
    Class,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::EnumVariant => "enum_variant",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Use => "use",
            Self::Mod => "mod",
            Self::Const => "const",
            Self::Static => "static",
            Self::TypeAlias => "type_alias",
            Self::Macro => "macro",
            Self::Union => "union",
            Self::Interface => "interface",
            Self::Class => "class",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for SymbolKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "function" => Ok(Self::Function),
            "struct" => Ok(Self::Struct),
            "enum" => Ok(Self::Enum),
            "enum_variant" => Ok(Self::EnumVariant),
            "trait" => Ok(Self::Trait),
            "impl" => Ok(Self::Impl),
            "use" => Ok(Self::Use),
            "mod" => Ok(Self::Mod),
            "const" => Ok(Self::Const),
            "static" => Ok(Self::Static),
            "type_alias" => Ok(Self::TypeAlias),
            "macro" => Ok(Self::Macro),
            "union" => Ok(Self::Union),
            "interface" => Ok(Self::Interface),
            "class" => Ok(Self::Class),
            other => Err(format!("unknown symbol kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Visibility {
    Public,
    PublicCrate,
    Restricted,
    Private,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Public => "public",
            Self::PublicCrate => "pub(crate)",
            Self::Restricted => "restricted",
            Self::Private => "private",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for Visibility {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" => Ok(Self::Public),
            "pub(crate)" => Ok(Self::PublicCrate),
            "restricted" => Ok(Self::Restricted),
            "private" => Ok(Self::Private),
            other => Err(format!("unknown visibility: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub signature: String,
    pub doc_comment: Option<String>,
    pub body: Option<String>,
    pub details: Option<String>,
    pub attributes: Option<String>,
    pub impl_type: Option<String>,
}

fn parse_source(source: &str) -> Result<tree_sitter::Tree, String> {
    std::thread_local! {
        static PARSER: std::cell::RefCell<Option<Parser>> = const { std::cell::RefCell::new(None) };
    }
    PARSER.with_borrow_mut(|slot| {
        let parser = slot.get_or_insert_with(|| {
            let mut p = Parser::new();
            let _ = p.set_language(&tree_sitter_rust::LANGUAGE.into());
            p
        });
        parser
            .parse(source, None)
            .ok_or_else(|| "Failed to parse source".to_string())
    })
}

pub fn parse_rust_source(
    source: &str,
    file_path: &str,
) -> Result<(Vec<Symbol>, Vec<TraitImpl>), String> {
    let tree = parse_source(source)?;
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

pub(crate) fn line_range(node: &Node) -> (usize, usize) {
    (node.start_position().row + 1, node.end_position().row + 1)
}

fn extract_doc_comment(node: &Node, source: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        match sib.kind() {
            "line_comment" => {
                let text = node_text(&sib, source);
                if let Some(stripped) = text.strip_prefix("///") {
                    let stripped = stripped.strip_prefix(' ').unwrap_or(stripped);
                    lines.push(stripped.trim_end().to_string());
                } else {
                    break;
                }
            }
            "block_comment" => {
                let text = node_text(&sib, source);
                if let Some(inner) = text.strip_prefix("/**") {
                    let inner = inner.strip_suffix("*/").unwrap_or(inner);
                    lines.push(inner.trim().to_string());
                } else {
                    break;
                }
            }
            "attribute_item" => {
                // Skip attributes like #[derive(...)]
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

fn extract_attributes(node: &Node, source: &str) -> Option<String> {
    let mut attrs = Vec::new();
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        match sib.kind() {
            "attribute_item" => {
                let text = node_text(&sib, source);
                let inner = text
                    .strip_prefix("#[")
                    .and_then(|s| s.strip_suffix(']'))
                    .unwrap_or(&text);
                attrs.push(inner.trim().to_string());
            }
            "line_comment" | "block_comment" => {}
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

pub(crate) fn extract_body(node: &Node, source: &str) -> String {
    let text = &source[node.start_byte()..node.end_byte()];
    let line_count = text.lines().count();
    if line_count > 100 {
        let lines: Vec<&str> = text.lines().collect();
        let head_count = 50;
        let tail_count = 10;
        let omitted = line_count - head_count - tail_count;
        let head: String = lines[..head_count].join("\n");
        let tail: String = lines[line_count - tail_count..].join("\n");
        format!("{head}\n// ... {omitted} lines omitted ...\n{tail}")
    } else {
        text.to_string()
    }
}

fn extract_impl_block(
    node: &Node,
    source: &str,
    file_path: &str,
    symbols: &mut Vec<Symbol>,
    trait_impls: &mut Vec<TraitImpl>,
) {
    let type_name = extract_impl_type(node, source);
    let vis = get_visibility(node, source);
    let sig = get_signature(node, source);
    if let Some(ti) = extract_trait_impl_info(node, source, file_path) {
        trait_impls.push(ti);
    }
    let (line_start, line_end) = line_range(node);
    symbols.push(Symbol {
        name: type_name.clone().unwrap_or_default(),
        kind: SymbolKind::Impl,
        visibility: vis,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: sig,
        doc_comment: None,
        body: Some(extract_body(node, source)),
        details: None,
        attributes: None,
        impl_type: None,
    });
    extract_symbols(
        node,
        source,
        file_path,
        type_name.as_deref(),
        symbols,
        trait_impls,
    );
}

fn extract_symbols(
    node: &Node,
    source: &str,
    file_path: &str,
    impl_type_name: Option<&str>,
    symbols: &mut Vec<Symbol>,
    trait_impls: &mut Vec<TraitImpl>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(mut sym) = extract_function(&child, source, file_path) {
                    sym.impl_type = impl_type_name.map(String::from);
                    symbols.push(sym);
                }
            }
            "struct_item" | "enum_item" | "trait_item" | "mod_item" | "const_item"
            | "static_item" | "type_item" | "union_item" => {
                let kind = match child.kind() {
                    "struct_item" => SymbolKind::Struct,
                    "enum_item" => SymbolKind::Enum,
                    "trait_item" => SymbolKind::Trait,
                    "mod_item" => SymbolKind::Mod,
                    "const_item" => SymbolKind::Const,
                    "static_item" => SymbolKind::Static,
                    "type_item" => SymbolKind::TypeAlias,
                    "union_item" => SymbolKind::Union,
                    _ => continue,
                };
                if let Some(sym) = extract_named_item(&child, source, file_path, kind) {
                    if child.kind() == "enum_item" {
                        extract_enum_variants(
                            &child,
                            source,
                            file_path,
                            &sym.name,
                            sym.visibility,
                            symbols,
                        );
                    }
                    // Extract derive-generated trait impls
                    if matches!(child.kind(), "struct_item" | "enum_item" | "union_item") {
                        extract_derive_trait_impls(&sym, trait_impls);
                    }
                    // Recurse into inline module bodies
                    if child.kind() == "mod_item"
                        && let Some(body) = find_child_by_kind(&child, "declaration_list")
                    {
                        extract_symbols(&body, source, file_path, None, symbols, trait_impls);
                    }
                    symbols.push(sym);
                }
            }
            "impl_item" => {
                extract_impl_block(&child, source, file_path, symbols, trait_impls);
            }
            "use_declaration" => {
                let text = node_text(&child, source);
                let (line_start, line_end) = line_range(&child);
                symbols.push(Symbol {
                    name: text.clone(),
                    kind: SymbolKind::Use,
                    visibility: get_visibility(&child, source),
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
            "function_signature_item" => {
                if let Some(mut sym) = extract_function_signature(&child, source, file_path) {
                    sym.impl_type = impl_type_name.map(String::from);
                    symbols.push(sym);
                }
            }
            "macro_definition" => {
                if let Some(sym) = extract_macro_def(&child, source, file_path) {
                    symbols.push(sym);
                }
            }
            "foreign_mod_item" | "declaration_list" => {
                extract_symbols(
                    &child,
                    source,
                    file_path,
                    impl_type_name,
                    symbols,
                    trait_impls,
                );
            }
            _ => {}
        }
    }
}

fn extract_function(node: &Node, source: &str, file_path: &str) -> Option<Symbol> {
    let name = find_child_by_kind(node, "identifier")?;
    let name_text = node_text(&name, source);
    let vis = get_visibility(node, source);
    let sig = get_signature(node, source);
    let (line_start, line_end) = line_range(node);

    Some(Symbol {
        name: name_text,
        kind: SymbolKind::Function,
        visibility: vis,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: sig,
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details: None,
        attributes: extract_attributes(node, source),
        impl_type: None,
    })
}

fn extract_function_signature(node: &Node, source: &str, file_path: &str) -> Option<Symbol> {
    let name = find_child_by_kind(node, "identifier")?;
    let name_text = node_text(&name, source);
    let vis = get_visibility(node, source);
    let sig = get_signature(node, source);
    let (line_start, line_end) = line_range(node);

    Some(Symbol {
        name: name_text,
        kind: SymbolKind::Function,
        visibility: vis,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: sig,
        doc_comment: extract_doc_comment(node, source),
        body: None,
        details: None,
        attributes: extract_attributes(node, source),
        impl_type: None,
    })
}

fn extract_struct_details(node: &Node, source: &str) -> Option<String> {
    // Named fields: struct Foo { pub x: i32, y: String }
    if let Some(list) = find_child_by_kind(node, "field_declaration_list") {
        let mut fields = Vec::new();
        let mut cursor = list.walk();
        for child in list.children(&mut cursor) {
            if child.kind() == "field_declaration" {
                let text = node_text(&child, source).trim_end().to_string();
                let text = text.strip_suffix(',').unwrap_or(&text).to_string();
                fields.push(text);
            }
        }
        if !fields.is_empty() {
            return Some(fields.join("\n"));
        }
    }
    // Tuple fields: struct Pair(pub u32, pub String)
    if let Some(list) = find_child_by_kind(node, "ordered_field_declaration_list") {
        let text = node_text(&list, source);
        // Strip outer parens: "(pub u32, pub String)" → "pub u32, pub String"
        let inner = text
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(&text);
        let fields: Vec<&str> = inner
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if !fields.is_empty() {
            let details: Vec<String> = fields
                .iter()
                .enumerate()
                .map(|(i, f)| format!("{i}: {f}"))
                .collect();
            return Some(details.join("\n"));
        }
    }
    None
}

fn extract_enum_details(node: &Node, source: &str) -> Option<String> {
    let list = find_child_by_kind(node, "enum_variant_list")?;
    let mut variants = Vec::new();
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.kind() == "enum_variant" {
            let text = node_text(&child, source).trim_end().to_string();
            let text = text.strip_suffix(',').unwrap_or(&text).to_string();
            variants.push(text);
        }
    }
    if variants.is_empty() {
        return None;
    }
    Some(variants.join("\n"))
}

fn extract_enum_variants(
    enum_node: &Node,
    source: &str,
    file_path: &str,
    enum_name: &str,
    enum_vis: Visibility,
    symbols: &mut Vec<Symbol>,
) {
    let Some(list) = find_child_by_kind(enum_node, "enum_variant_list") else {
        return;
    };
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.kind() == "enum_variant" {
            let Some(name_node) = find_child_by_kind(&child, "identifier") else {
                continue;
            };
            let variant_name = node_text(&name_node, source);
            let text = node_text(&child, source).trim_end().to_string();
            let text = text.strip_suffix(',').unwrap_or(&text).to_string();
            let (line_start, line_end) = line_range(&child);
            symbols.push(Symbol {
                name: variant_name,
                kind: SymbolKind::EnumVariant,
                visibility: enum_vis,
                file_path: file_path.to_string(),
                line_start,
                line_end,
                signature: text,
                doc_comment: extract_doc_comment(&child, source),
                body: None,
                details: None,
                attributes: extract_attributes(&child, source),
                impl_type: Some(enum_name.to_string()),
            });
        }
    }
}

fn extract_trait_details(node: &Node, source: &str) -> Option<String> {
    let list = find_child_by_kind(node, "declaration_list")?;
    let mut items = Vec::new();
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        match child.kind() {
            "function_signature_item" | "function_item" | "const_item" => {
                let sig = get_signature(&child, source);
                let sig = sig.strip_suffix(';').unwrap_or(&sig).to_string();
                items.push(sig);
            }
            "associated_type" => {
                let text = node_text(&child, source).trim_end().to_string();
                let text = text.strip_suffix(';').unwrap_or(&text).to_string();
                items.push(text);
            }
            _ => {}
        }
    }
    if items.is_empty() {
        return None;
    }
    Some(items.join("\n"))
}

fn extract_named_item(
    node: &Node,
    source: &str,
    file_path: &str,
    kind: SymbolKind,
) -> Option<Symbol> {
    let name_node = find_child_by_kind(node, "type_identifier")
        .or_else(|| find_child_by_kind(node, "identifier"))?;
    let name = node_text(&name_node, source);
    let vis = get_visibility(node, source);
    let sig = get_signature(node, source);

    let details = match kind {
        SymbolKind::Struct | SymbolKind::Union => extract_struct_details(node, source),
        SymbolKind::Enum => extract_enum_details(node, source),
        SymbolKind::Trait => extract_trait_details(node, source),
        _ => None,
    };
    let (line_start, line_end) = line_range(node);

    Some(Symbol {
        name,
        kind,
        visibility: vis,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: sig,
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details,
        attributes: extract_attributes(node, source),
        impl_type: None,
    })
}

fn extract_macro_def(node: &Node, source: &str, file_path: &str) -> Option<Symbol> {
    let name = find_child_by_kind(node, "identifier")?;
    let name_text = node_text(&name, source);
    let sig = get_signature(node, source);
    let (line_start, line_end) = line_range(node);

    Some(Symbol {
        name: name_text,
        kind: SymbolKind::Macro,
        visibility: Visibility::Public,
        file_path: file_path.to_string(),
        line_start,
        line_end,
        signature: sig,
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details: None,
        attributes: extract_attributes(node, source),
        impl_type: None,
    })
}

fn extract_impl_type(node: &Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "type_identifier" || child.kind() == "generic_type")
        .map(|child| node_text(&child, source))
}

fn is_type_node(kind: &str) -> bool {
    kind == "type_identifier" || kind == "scoped_type_identifier" || kind == "generic_type"
}

fn extract_type_name(node: &Node, source: &str) -> String {
    if node.kind() == "generic_type"
        && let Some(base) = find_child_by_kind(node, "type_identifier")
    {
        return node_text(&base, source);
    }
    node_text(node, source)
}

fn extract_trait_impl_info(node: &Node, source: &str, file_path: &str) -> Option<TraitImpl> {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();

    let for_pos = children.iter().position(|c| c.kind() == "for")?;

    let trait_node = children[..for_pos]
        .iter()
        .rfind(|c| is_type_node(c.kind()))?;

    let type_node = children[for_pos + 1..]
        .iter()
        .find(|c| is_type_node(c.kind()))?;

    let (line_start, line_end) = line_range(node);
    Some(TraitImpl {
        trait_name: extract_type_name(trait_node, source),
        type_name: extract_type_name(type_node, source),
        file_path: file_path.to_string(),
        line_start,
        line_end,
    })
}

/// Known derive macros that map to specific trait names.
/// `thiserror::Error` implies both `Error` and `Display`.
const DERIVE_TRAIT_MAP: &[(&str, &[&str])] = &[
    ("Error", &["Error", "Display"]),
    ("thiserror::Error", &["Error", "Display"]),
    ("thiserror::error::Error", &["Error", "Display"]),
];

/// Extract synthetic `TraitImpl` entries from `#[derive(...)]` attributes.
pub fn extract_derive_trait_impls(sym: &Symbol, trait_impls: &mut Vec<TraitImpl>) {
    let Some(attrs) = &sym.attributes else {
        return;
    };

    // Find derive(...) blocks in the attributes string.
    // Attributes are joined with ", " but derive contents also have commas,
    // so we search for "derive(" and find the matching ")".
    let mut search_from = 0;
    while let Some(start) = attrs[search_from..].find("derive(") {
        let abs_start = search_from + start + 7; // skip "derive("
        let Some(end) = attrs[abs_start..].find(')') else {
            break;
        };
        let inner = &attrs[abs_start..abs_start + end];
        search_from = abs_start + end + 1;

        // inner is now e.g. "Debug, Clone, Serialize"
        for raw_trait in inner.split(',') {
            let trait_name = raw_trait.trim();
            if trait_name.is_empty() {
                continue;
            }
            // Check if this derive maps to additional traits
            let mut found_mapping = false;
            for (key, implied_traits) in DERIVE_TRAIT_MAP {
                if trait_name == *key {
                    for t in *implied_traits {
                        trait_impls.push(TraitImpl {
                            trait_name: (*t).to_string(),
                            type_name: sym.name.clone(),
                            file_path: sym.file_path.clone(),
                            line_start: sym.line_start,
                            line_end: sym.line_end,
                        });
                    }
                    found_mapping = true;
                    break;
                }
            }
            if !found_mapping {
                // Strip module path: serde::Serialize → Serialize
                let short = trait_name.rsplit("::").next().unwrap_or(trait_name);
                trait_impls.push(TraitImpl {
                    trait_name: short.to_string(),
                    type_name: sym.name.clone(),
                    file_path: sym.file_path.clone(),
                    line_start: sym.line_start,
                    line_end: sym.line_end,
                });
            }
        }
    }
}

fn get_visibility(node: &Node, source: &str) -> Visibility {
    let Some(vis_node) = find_child_by_kind(node, "visibility_modifier") else {
        return Visibility::Private;
    };
    let text = node_text(&vis_node, source);
    if text == "pub" {
        return Visibility::Public;
    }
    // pub(...) forms: check what's inside the parens
    // "pub(in ...)" must be checked first since "pub(in crate::x)"
    // contains "crate" but is a path restriction, not pub(crate)
    if text.contains("super") || text.contains("self") || text.contains("in ") {
        Visibility::Restricted
    } else if text.contains("crate") {
        Visibility::PublicCrate
    } else {
        Visibility::Public
    }
}

pub(crate) fn find_child_by_kind<'a>(node: &'a Node, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

pub(crate) fn node_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

pub(crate) fn get_signature(node: &Node, source: &str) -> String {
    let text = node_text(node, source);
    let sig_end = text.find('{').unwrap_or(text.len());
    let raw_sig = text[..sig_end].trim();
    raw_sig.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Resolved import path for a short name.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImportInfo {
    pub qualified_path: String,
}

/// A reference from one symbol to another, identified by name.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SymbolRef {
    /// Name of the symbol that contains the reference
    pub source_name: String,
    /// File path of the source symbol
    pub source_file: String,
    /// Name of the referenced symbol
    pub target_name: String,
    /// Kind of reference
    pub kind: RefKind,
    /// Resolved target file path from import map (if available)
    pub target_file: Option<String>,
    /// Impl type context for `self.method()` calls
    pub target_context: Option<String>,
    /// Line where the reference occurs in the source file (1-based)
    pub ref_line: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum RefKind {
    /// Type used in signature or body
    TypeRef,
    /// Function/method call
    Call,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TraitImpl {
    pub type_name: String,
    pub trait_name: String,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
}

const NOISY_SYMBOL_NAMES: &[&str] = &[
    // Prelude types (always available, never imported)
    "Box",
    "Err",
    "None",
    "Ok",
    "Option",
    "Result",
    "Some",
    "String",
    "Vec",
    // Standard trait implementations
    "as_mut",
    "as_ref",
    "borrow",
    "borrow_mut",
    "cmp",
    "deref",
    "deref_mut",
    "drop",
    "eq",
    "fmt",
    "hash",
    "ne",
    "partial_cmp",
    "to_owned",
    "to_string",
    "try_from",
    "try_into",
    // Common collection/iterator methods
    "capacity",
    "clear",
    "collect",
    "contains",
    "extend",
    "filter",
    "get",
    "insert",
    "into_iter",
    "is_empty",
    "iter",
    "len",
    "map",
    "pop",
    "push",
    "remove",
    "set",
    "with_capacity",
    // Display and formatting
    "debug",
    "display",
    "eprint",
    "eprintln",
    "format",
    "print",
    "println",
    "write",
    "writeln",
    // Common conversions
    "expect",
    "unwrap",
];

fn is_noisy_symbol(name: &str) -> bool {
    NOISY_SYMBOL_NAMES.contains(&name)
}

/// Derive a target file path from a module-qualified call like `parser::foo`.
/// For `src/indexer/mod.rs` calling `parser::foo`, returns `src/indexer/parser.rs`.
fn module_to_file(current_file: &str, module_name: &str) -> Option<String> {
    sibling_file(current_file, &format!("{module_name}.rs"))
}

/// Resolve `super::` to the parent module's file.
/// `src/server/tools/impact.rs` -> `src/server/tools/mod.rs`
fn super_to_file(current_file: &str) -> Option<String> {
    sibling_file(current_file, "mod.rs")
}

/// Join `file_name` to the parent directory of `current_file`, returning
/// the result as a UTF-8 string (lossy if needed). Shared by the
/// module-path resolvers in this file.
fn sibling_file(current_file: &str, file_name: &str) -> Option<String> {
    let parent = std::path::Path::new(current_file).parent()?;
    Some(parent.join(file_name).to_string_lossy().to_string())
}

/// Extract type context from a simple scoped identifier like `Database::new`.
/// Returns the type/module name if the first child is an identifier or type identifier.
fn extract_scoped_context(node: &Node, source: &str) -> Option<String> {
    if node.child_count() < 3 {
        return None;
    }
    let first = node.child(0)?;
    let kind = first.kind();
    if kind == "identifier" || kind == "type_identifier" {
        Some(node_text(&first, source))
    } else {
        None
    }
}

const CONSTRUCTOR_NAMES: &[&str] = &["new", "from", "into", "default", "clone", "build", "init"];

fn is_constructor_name(name: &str) -> bool {
    CONSTRUCTOR_NAMES.contains(&name)
}

impl std::fmt::Display for RefKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TypeRef => f.write_str("type_ref"),
            Self::Call => f.write_str("call"),
        }
    }
}

/// Convert a qualified path to candidate relative file paths, resolving both
/// `crate::` prefixes and workspace crate prefixes via `crate_map`.
/// Returns both `module.rs` and `module/mod.rs` variants for each path.
fn qualified_path_to_files_with_crates<S: std::hash::BuildHasher>(
    qualified_path: &str,
    crate_map: &std::collections::HashMap<String, String, S>,
) -> Vec<String> {
    // crate:: prefix (current crate)
    if let Some(path) = qualified_path.strip_prefix("crate::") {
        let segments: Vec<&str> = path.split("::").collect();
        if segments.len() < 2 {
            return Vec::new();
        }
        let module_segments = &segments[..segments.len() - 1];
        let joined = module_segments.join("/");
        return vec![format!("src/{joined}.rs"), format!("src/{joined}/mod.rs")];
    }

    // Workspace crate prefixes
    let segments: Vec<&str> = qualified_path.split("::").collect();
    if segments.len() < 2 {
        return Vec::new();
    }
    let crate_name = segments[0];
    let Some(crate_path) = crate_map.get(crate_name) else {
        return Vec::new();
    };

    // Normalize "." to "" so paths don't start with "./"
    let prefix = if crate_path == "." {
        ""
    } else {
        crate_path.as_str()
    };
    let sep = if prefix.is_empty() { "" } else { "/" };

    if segments.len() == 2 {
        return vec![format!("{prefix}{sep}src/lib.rs")];
    }
    let module_segments = &segments[1..segments.len() - 1];
    let joined = module_segments.join("/");
    vec![
        format!("{prefix}{sep}src/{joined}.rs"),
        format!("{prefix}{sep}src/{joined}/mod.rs"),
    ]
}

/// Convert a `crate::` qualified path to a relative file path (first candidate).
#[cfg(test)]
fn qualified_path_to_file(qualified_path: &str) -> Option<String> {
    qualified_path_to_files_with_crates(
        qualified_path,
        &std::collections::HashMap::<String, String>::new(),
    )
    .into_iter()
    .next()
}

/// Build an import map from `use` declarations in the given AST root.
///
/// Maps short imported names to their fully qualified paths.
/// Glob imports (`use foo::*`) are skipped.
#[must_use]
pub fn extract_import_map(
    root: &Node,
    source: &str,
) -> std::collections::HashMap<String, ImportInfo> {
    let mut map = std::collections::HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "use_declaration" {
            continue;
        }
        let mut inner_cursor = child.walk();
        for use_child in child.children(&mut inner_cursor) {
            collect_use_entries(&use_child, source, "", &mut map);
        }
    }
    map
}

fn collect_use_entries(
    node: &Node,
    source: &str,
    prefix: &str,
    map: &mut std::collections::HashMap<String, ImportInfo>,
) {
    match node.kind() {
        "use_as_clause" => {
            // `use path::Name as Alias;`
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            // The path is the first child (scoped_identifier or identifier),
            // "as" keyword, then the alias identifier
            let path_text = if let Some(first) = children.first() {
                let text = node_text(first, source);
                if prefix.is_empty() {
                    text
                } else {
                    format!("{prefix}::{text}")
                }
            } else {
                return;
            };
            if let Some(alias_node) = children.last()
                && alias_node.kind() == "identifier"
            {
                let alias = node_text(alias_node, source);
                map.insert(
                    alias,
                    ImportInfo {
                        qualified_path: path_text,
                    },
                );
            }
        }
        "scoped_identifier" => {
            // Direct import like `use crate::config::Config;`
            let text = node_text(node, source);
            let full_path = if prefix.is_empty() {
                text.clone()
            } else {
                format!("{prefix}::{text}")
            };
            if let Some(short_name) = text.rsplit("::").next() {
                map.insert(
                    short_name.to_string(),
                    ImportInfo {
                        qualified_path: full_path,
                    },
                );
            }
        }
        "identifier" => {
            // Bare identifier (inside a use_list or standalone)
            let name = node_text(node, source);
            let full_path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}::{name}")
            };
            map.insert(
                name,
                ImportInfo {
                    qualified_path: full_path,
                },
            );
        }
        "scoped_use_list" => {
            // `use path::{A, B};`
            // Find the path prefix and the use_list
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            let mut path_prefix = String::new();
            for c in &children {
                match c.kind() {
                    "identifier" | "scoped_identifier" | "self" => {
                        let seg = node_text(c, source);
                        path_prefix = if prefix.is_empty() {
                            seg
                        } else {
                            format!("{prefix}::{seg}")
                        };
                    }
                    "use_list" => {
                        let mut list_cursor = c.walk();
                        for item in c.children(&mut list_cursor) {
                            collect_use_entries(&item, source, &path_prefix, map);
                        }
                    }
                    _ => {}
                }
            }
        }
        "use_list" => {
            // Top-level use_list (shouldn't normally happen at root)
            let mut cursor = node.walk();
            for item in node.children(&mut cursor) {
                collect_use_entries(&item, source, prefix, map);
            }
        }
        _ => {}
    }
}

/// Convenience wrapper: parse source and extract the import map.
#[cfg(test)]
fn extract_import_map_from_source(
    source: &str,
) -> Result<std::collections::HashMap<String, ImportInfo>, String> {
    let tree = parse_source(source)?;
    let root = tree.root_node();
    Ok(extract_import_map(&root, source))
}

/// Shared context for reference extraction, avoiding excessive
/// parameter passing through the recursive call chain.
struct RefContext<'a, S: std::hash::BuildHasher, S2: std::hash::BuildHasher> {
    source: &'a str,
    file_path: &'a str,
    known_symbols: &'a std::collections::HashSet<String, S>,
    import_map: std::collections::HashMap<String, ImportInfo>,
    crate_map: &'a std::collections::HashMap<String, String, S2>,
}

/// Extract references between symbols by scanning function/impl bodies
/// for identifiers that match known symbol names.
pub fn extract_refs<S: std::hash::BuildHasher, S2: std::hash::BuildHasher>(
    source: &str,
    file_path: &str,
    known_symbols: &std::collections::HashSet<String, S>,
    crate_map: &std::collections::HashMap<String, String, S2>,
) -> Result<Vec<SymbolRef>, String> {
    let tree = parse_source(source)?;
    let root = tree.root_node();
    let import_map = extract_import_map(&root, source);
    let ctx = RefContext {
        source,
        file_path,
        known_symbols,
        import_map,
        crate_map,
    };
    let mut refs = Vec::new();
    collect_refs(&root, &ctx, None, &mut refs);
    Ok(refs)
}

fn collect_refs<S: std::hash::BuildHasher, S2: std::hash::BuildHasher>(
    node: &Node,
    ctx: &RefContext<'_, S, S2>,
    impl_type: Option<&str>,
    refs: &mut Vec<SymbolRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                let Some(name_node) = find_child_by_kind(&child, "identifier") else {
                    continue;
                };
                let fn_name = node_text(&name_node, ctx.source);
                collect_body_refs(&child, &fn_name, impl_type, ctx, refs);
            }
            "impl_item" => {
                let type_name = extract_impl_type(&child, ctx.source);
                collect_refs(&child, ctx, type_name.as_deref(), refs);
            }
            "declaration_list" => {
                collect_refs(&child, ctx, impl_type, refs);
            }
            "mod_item" => {
                collect_refs(&child, ctx, None, refs);
            }
            "struct_item" | "enum_item" => {
                collect_derive_refs(&child, ctx, refs);
                collect_field_refs(&child, ctx, refs);
            }
            _ => {}
        }
    }
}

/// Extract refs from `#[derive(Trait1, Trait2)]` attributes on a symbol.
fn collect_derive_refs<S: std::hash::BuildHasher, S2: std::hash::BuildHasher>(
    node: &Node,
    ctx: &RefContext<'_, S, S2>,
    refs: &mut Vec<SymbolRef>,
) {
    let name_node = find_child_by_kind(node, "type_identifier")
        .or_else(|| find_child_by_kind(node, "identifier"));
    let Some(name_node) = name_node else { return };
    let source_name = node_text(&name_node, ctx.source);

    let Some(attrs) = extract_attributes(node, ctx.source) else {
        return;
    };
    for attr in attrs.split(", ") {
        let Some(inner) = attr
            .strip_prefix("derive(")
            .and_then(|s| s.strip_suffix(')'))
        else {
            continue;
        };
        for derive_name in inner.split(',') {
            let derive_name = derive_name.trim();
            // Handle path-qualified derives: serde::Serialize → Serialize
            let derive_name = derive_name.rsplit("::").next().unwrap_or(derive_name);
            if !derive_name.is_empty() && ctx.known_symbols.contains(derive_name) {
                refs.push(SymbolRef {
                    source_name: source_name.clone(),
                    source_file: ctx.file_path.to_string(),
                    target_name: derive_name.to_string(),
                    kind: RefKind::TypeRef,
                    target_file: resolve_target_file(derive_name, ctx),
                    target_context: None,
                    ref_line: i64::try_from(node.start_position().row + 1).ok(),
                });
            }
        }
    }
}

/// Extract `TypeRef` refs from the field types of a struct or the variant
/// payload types of an enum. Without this, a type used only as a struct field
/// (the common case for data-container types) has zero entries in
/// `symbol_refs` and is therefore invisible to `impact`, which filters on
/// `high`-confidence refs.
///
/// The `seen` set is shared across the entire subtree walk (generics,
/// nested `Vec<Option<T>>`, trait bounds, etc.), so each target symbol
/// contributes at most one ref per source struct/enum — a struct that
/// mentions the same type in three fields still emits one `TypeRef`.
fn collect_field_refs<S: std::hash::BuildHasher, S2: std::hash::BuildHasher>(
    node: &Node,
    ctx: &RefContext<'_, S, S2>,
    refs: &mut Vec<SymbolRef>,
) {
    let name_node = find_child_by_kind(node, "type_identifier")
        .or_else(|| find_child_by_kind(node, "identifier"));
    let Some(name_node) = name_node else { return };
    let source_name = node_text(&name_node, ctx.source);

    let body = match node.kind() {
        "struct_item" => find_child_by_kind(node, "field_declaration_list")
            .or_else(|| find_child_by_kind(node, "ordered_field_declaration_list")),
        "enum_item" => find_child_by_kind(node, "enum_variant_list"),
        _ => None,
    };
    let Some(body) = body else { return };

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stack = vec![body];
    while let Some(n) = stack.pop() {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                let name = node_text(&child, ctx.source);
                if name != source_name
                    && ctx.known_symbols.contains(&name)
                    && seen.insert(name.clone())
                {
                    let line = i64::try_from(child.start_position().row + 1).ok();
                    refs.push(SymbolRef {
                        source_name: source_name.clone(),
                        source_file: ctx.file_path.to_string(),
                        target_name: name.clone(),
                        kind: RefKind::TypeRef,
                        target_file: resolve_target_file(&name, ctx),
                        target_context: None,
                        ref_line: line,
                    });
                }
            } else {
                stack.push(child);
            }
        }
    }
}

/// Recursively collect all bound identifiers from a pattern node.
/// Handles tuple patterns `(a, b)`, struct patterns `Foo { x, y }`, etc.
fn collect_pattern_identifiers(
    node: &Node,
    source: &str,
    locals: &mut std::collections::HashSet<String>,
) {
    let mut stack = vec![*node];
    while let Some(n) = stack.pop() {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "identifier" {
                locals.insert(node_text(&child, source));
            } else {
                stack.push(child);
            }
        }
    }
}

fn collect_locals(node: &Node, source: &str) -> std::collections::HashSet<String> {
    let mut locals = std::collections::HashSet::new();
    let mut stack = vec![*node];

    while let Some(n) = stack.pop() {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            match child.kind() {
                "parameter" | "self_parameter" | "let_declaration" | "for_expression" => {
                    if let Some(pat) = find_child_by_kind(&child, "identifier") {
                        locals.insert(node_text(&pat, source));
                    }
                }
                "closure_parameters" => {
                    collect_pattern_identifiers(&child, source, &mut locals);
                }
                _ => {}
            }
            // Always recurse except into parameter/self_parameter
            // (they have no deeper bindings)
            if child.kind() != "parameter" && child.kind() != "self_parameter" {
                stack.push(child);
            }
        }
    }
    locals
}

/// Extract the base type name from a type AST node, stripping references
/// and extracting the outermost type from generics.
/// `&Database` → `Database`, `&mut Config` → `Config`,
/// `Vec<String>` → `Vec`, `Option<&str>` → `Option`.
fn extract_type_from_node(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(node_text(node, source)),
        "scoped_type_identifier" => {
            // e.g. std::io::Result → last segment
            let text = node_text(node, source);
            text.rsplit("::").next().map(String::from)
        }
        "reference_type" => {
            // &T or &mut T → recurse to find the inner type
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(t) = extract_type_from_node(&child, source) {
                    return Some(t);
                }
            }
            None
        }
        "generic_type" => {
            // Vec<T> → "Vec" (the outer type)
            find_child_by_kind(node, "type_identifier").map(|n| node_text(&n, source))
        }
        "pointer_type" => {
            // *mut T or *const T → recurse to inner type
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(t) = extract_type_from_node(&child, source) {
                    return Some(t);
                }
            }
            None
        }
        "array_type" | "slice_type" => {
            // [T; N] or [T] → extract inner type
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(t) = extract_type_from_node(&child, source) {
                    return Some(t);
                }
            }
            None
        }
        "dynamic_type" => {
            // dyn Trait → extract the trait name
            find_child_by_kind(node, "type_identifier").map(|n| node_text(&n, source))
        }
        _ => None,
    }
}

/// Infer the type of a let-binding's value expression.
/// Handles: `Type::method(...)`, `Type { ... }`, and `Type(...)`.
fn infer_type_from_value(node: &Node, source: &str) -> Option<String> {
    // Unwrap try expressions: `expr?` → look at inner expr
    let inner = if node.kind() == "try_expression" {
        node.child(0)?
    } else {
        *node
    };

    match inner.kind() {
        "call_expression" => {
            // Type::new(...) → look at the function part
            let func = inner.child(0)?;
            infer_type_from_call_func(&func, source)
        }
        "struct_expression" => {
            // Config { field: value } → get the type name
            let mut cursor = inner.walk();
            for child in inner.children(&mut cursor) {
                if let Some(t) = extract_type_from_node(&child, source) {
                    return Some(t);
                }
            }
            None
        }
        _ => None,
    }
}

/// Extract the type from the function part of a call expression.
/// `Database::open` → `Database`, `Self::new` → `None` (handled by `impl_type`).
fn infer_type_from_call_func(node: &Node, source: &str) -> Option<String> {
    if node.kind() == "scoped_identifier" {
        // Type::method — first child is the type
        let first = node.child(0)?;
        match first.kind() {
            "type_identifier" => Some(node_text(&first, source)),
            "identifier" => {
                // Could be a module path like `module::func`,
                // only treat as type if it looks like UpperCamelCase
                let name = node_text(&first, source);
                if name.chars().next().is_some_and(char::is_uppercase) {
                    Some(name)
                } else {
                    None
                }
            }
            "scoped_identifier" => {
                // Nested: std::fs::File::open → get "File"
                // The last type_identifier before :: is the type
                let text = node_text(&first, source);
                for segment in text.rsplit("::") {
                    if segment.chars().next().is_some_and(char::is_uppercase) {
                        return Some(segment.to_string());
                    }
                }
                None
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Build a mapping of local variable names to their inferred type names.
/// Sources: function parameters (type annotations), let bindings
/// (type annotations and constructor inference), and `self` → `impl_type`.
fn collect_local_types(
    fn_node: &Node,
    source: &str,
    impl_type: Option<&str>,
) -> std::collections::HashMap<String, String> {
    let mut types = std::collections::HashMap::new();

    // `self` always maps to impl_type
    if let Some(it) = impl_type {
        types.insert("self".to_string(), it.to_string());
    }

    // Extract types from function parameters
    if let Some(params) = find_child_by_kind(fn_node, "parameters") {
        let mut cursor = params.walk();
        for param in params.children(&mut cursor) {
            if param.kind() == "parameter" {
                let name = find_child_by_kind(&param, "identifier").map(|n| node_text(&n, source));
                let Some(name) = name else { continue };

                // Look for type annotation on the parameter
                let mut inner = param.walk();
                for child in param.children(&mut inner) {
                    if let Some(t) = extract_type_from_node(&child, source) {
                        types.insert(name, t);
                        break;
                    }
                }
            }
        }
    }

    // Walk the function body for let bindings
    let mut stack = vec![*fn_node];
    while let Some(n) = stack.pop() {
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "let_declaration" {
                let name = find_child_by_kind(&child, "identifier").map(|n| node_text(&n, source));
                let Some(name) = name else { continue };

                // Strategy 1: explicit type annotation
                let mut found = false;
                let mut inner = child.walk();
                for c in child.children(&mut inner) {
                    if let Some(t) = extract_type_from_node(&c, source) {
                        types.insert(name.clone(), t);
                        found = true;
                        break;
                    }
                }

                // Strategy 2: infer from value expression
                if !found {
                    let mut inner = child.walk();
                    for c in child.children(&mut inner) {
                        if c.kind() == "call_expression"
                            || c.kind() == "struct_expression"
                            || c.kind() == "try_expression"
                        {
                            if let Some(t) = infer_type_from_value(&c, source) {
                                types.insert(name.clone(), t);
                            }
                            break;
                        }
                    }
                }
            }
            // Recurse into blocks but not into nested functions
            if child.kind() != "function_item" && child.kind() != "closure_expression" {
                stack.push(child);
            }
        }
    }

    types
}

fn resolve_target_file<S: std::hash::BuildHasher, S2: std::hash::BuildHasher>(
    name: &str,
    ctx: &RefContext<'_, S, S2>,
) -> Option<String> {
    if let Some(info) = ctx.import_map.get(name) {
        let first = qualified_path_to_files_with_crates(&info.qualified_path, ctx.crate_map)
            .into_iter()
            .next();
        if first.is_some() {
            return first;
        }
    }
    if ctx.known_symbols.contains(name) {
        Some(ctx.file_path.to_string())
    } else {
        None
    }
}

struct BodyRefCollector<'a, S: std::hash::BuildHasher, S2: std::hash::BuildHasher> {
    fn_name: &'a str,
    ctx: &'a RefContext<'a, S, S2>,
    locals: std::collections::HashSet<String>,
    seen: std::collections::HashSet<String>,
}

impl<S: std::hash::BuildHasher, S2: std::hash::BuildHasher> BodyRefCollector<'_, S, S2> {
    fn try_add(
        &mut self,
        name: &str,
        kind: RefKind,
        target_context: Option<String>,
        line: Option<i64>,
        refs: &mut Vec<SymbolRef>,
    ) {
        // Qualified calls (with target_context) bypass the noisy filter —
        // `Status::clear()` is unambiguous unlike a bare `clear()`.
        let noisy = target_context.is_none() && is_noisy_symbol(name);
        if name != self.fn_name
            && !noisy
            && (target_context.is_some() || !self.locals.contains(name))
            && self.ctx.known_symbols.contains(name)
            && self.seen.insert(match &target_context {
                Some(ctx) => format!("{ctx}::{name}"),
                None => name.to_string(),
            })
        {
            refs.push(SymbolRef {
                source_name: self.fn_name.to_string(),
                source_file: self.ctx.file_path.to_string(),
                target_name: name.to_string(),
                kind,
                target_file: resolve_target_file(name, self.ctx),
                target_context,
                ref_line: line,
            });
        }
    }

    /// Add a ref from a fully-qualified `crate::` path.
    /// Bypasses noisy-symbol filter since the qualification is
    /// unambiguous and resolves the target file from the path.
    fn try_add_qualified(
        &mut self,
        name: &str,
        kind: RefKind,
        target_context: Option<String>,
        target_file: Option<String>,
        line: Option<i64>,
        refs: &mut Vec<SymbolRef>,
    ) {
        if name != self.fn_name
            && self.ctx.known_symbols.contains(name)
            && self.seen.insert(match &target_context {
                Some(ctx) => format!("{ctx}::{name}"),
                None => name.to_string(),
            })
        {
            refs.push(SymbolRef {
                source_name: self.fn_name.to_string(),
                source_file: self.ctx.file_path.to_string(),
                target_name: name.to_string(),
                kind,
                target_file,
                target_context,
                ref_line: line,
            });
        }
    }

    /// Handle a `crate::module::symbol` scoped identifier.
    /// Returns true if handled (caller should NOT descend).
    fn handle_crate_path(&mut self, child: &Node, refs: &mut Vec<SymbolRef>) -> bool {
        let text = node_text(child, self.ctx.source);
        if !text.starts_with("crate::") {
            return false;
        }
        let segments: Vec<&str> = text.split("::").collect();
        let Some(&final_name) = segments.last() else {
            return true;
        };
        let line = i64::try_from(child.start_position().row + 1).ok();
        let target_file = qualified_path_to_files_with_crates(&text, self.ctx.crate_map)
            .into_iter()
            .next();

        // Check if second-to-last segment is a type name
        let type_seg = segments
            .get(segments.len().wrapping_sub(2))
            .filter(|s| s.starts_with(|c: char| c.is_uppercase()));

        let target_context = type_seg.map(|s| (*s).to_string());

        if let Some(type_name) = type_seg {
            self.try_add_qualified(
                type_name,
                RefKind::TypeRef,
                None,
                target_file.clone(),
                line,
                refs,
            );
        }

        self.try_add_qualified(
            final_name,
            RefKind::Call,
            target_context,
            target_file,
            line,
            refs,
        );
        true
    }

    /// Handle a simple `Type::method` or `module::function` scoped identifier.
    /// Returns `true` if handled (caller should NOT descend).
    ///
    /// Lowercase qualifiers (e.g. `parser::extract_refs`) are treated as
    /// module paths: we derive the target file from the current file's
    /// directory so the ref gets file-qualified → high confidence.
    fn handle_scoped_call(&mut self, child: &Node, refs: &mut Vec<SymbolRef>) -> bool {
        // Handle super:: and self:: BEFORE extract_scoped_context,
        // because tree-sitter uses node kinds "super"/"self" (not "identifier"),
        // which extract_scoped_context doesn't handle.
        if let Some(first) = child.child(0) {
            let first_kind = first.kind();
            if first_kind == "super" || first_kind == "self" {
                let last_idx = u32::try_from(child.child_count().saturating_sub(1));
                if let Some(method_node) = last_idx.ok().and_then(|i| child.child(i)) {
                    let method_name = node_text(&method_node, self.ctx.source);
                    let line = i64::try_from(method_node.start_position().row + 1).ok();
                    let target_file = if first_kind == "self" {
                        Some(self.ctx.file_path.to_string())
                    } else {
                        super_to_file(self.ctx.file_path)
                    };
                    self.try_add_qualified(
                        &method_name,
                        RefKind::Call,
                        None,
                        target_file,
                        line,
                        refs,
                    );
                }
                return true;
            }
        }

        let Some(qualifier) = extract_scoped_context(child, self.ctx.source) else {
            return false;
        };
        let last_idx = u32::try_from(child.child_count().saturating_sub(1));
        if let Some(method_node) = last_idx.ok().and_then(|i| child.child(i)) {
            let method_name = node_text(&method_node, self.ctx.source);
            let line = i64::try_from(method_node.start_position().row + 1).ok();

            let is_module = qualifier.starts_with(|c: char| c.is_lowercase());
            if is_module {
                let target_file = module_to_file(self.ctx.file_path, &qualifier);
                self.try_add_qualified(&method_name, RefKind::Call, None, target_file, line, refs);
            } else if self.ctx.known_symbols.contains(&qualifier)
                || !is_constructor_name(&method_name)
            {
                self.try_add(&method_name, RefKind::Call, Some(qualifier), line, refs);
            }
        }
        true
    }
}

fn collect_body_refs<S: std::hash::BuildHasher, S2: std::hash::BuildHasher>(
    fn_node: &Node,
    fn_name: &str,
    impl_type: Option<&str>,
    ctx: &RefContext<'_, S, S2>,
    refs: &mut Vec<SymbolRef>,
) {
    let locals = collect_locals(fn_node, ctx.source);
    let local_types = collect_local_types(fn_node, ctx.source, impl_type);
    let mut col = BodyRefCollector {
        fn_name,
        ctx,
        locals,
        seen: std::collections::HashSet::new(),
    };
    let mut stack = vec![*fn_node];

    while let Some(node) = stack.pop() {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "type_identifier" | "identifier" => {
                    let name = node_text(&child, ctx.source);
                    let ref_kind = if child.kind() == "type_identifier" {
                        RefKind::TypeRef
                    } else {
                        RefKind::Call
                    };
                    let line = i64::try_from(child.start_position().row + 1).ok();
                    col.try_add(&name, ref_kind, None, line, refs);
                }
                "field_identifier" => {
                    let name = node_text(&child, ctx.source);
                    let target_context = child.parent().and_then(|p| {
                        if p.kind() != "field_expression" {
                            return None;
                        }
                        let receiver = p.child(0)?;
                        match receiver.kind() {
                            "self" => impl_type.map(String::from),
                            "identifier" => {
                                let var_name = node_text(&receiver, ctx.source);
                                local_types.get(&var_name).cloned()
                            }
                            _ => None,
                        }
                    });
                    let line = i64::try_from(child.start_position().row + 1).ok();
                    col.try_add(&name, RefKind::Call, target_context, line, refs);
                }
                "scoped_identifier" => {
                    if !col.handle_crate_path(&child, refs) && !col.handle_scoped_call(&child, refs)
                    {
                        stack.push(child);
                    }
                }
                "macro_invocation" => {
                    let mut macro_stack = vec![child];
                    while let Some(mn) = macro_stack.pop() {
                        let mut mc = mn.walk();
                        for mchild in mn.children(&mut mc) {
                            let mk = mchild.kind();
                            if mk == "type_identifier" || mk == "identifier" {
                                let name = node_text(&mchild, ctx.source);
                                let ref_kind = if mk == "type_identifier" {
                                    RefKind::TypeRef
                                } else {
                                    RefKind::Call
                                };
                                let line = i64::try_from(mchild.start_position().row + 1).ok();
                                col.try_add(&name, ref_kind, None, line, refs);
                            } else {
                                macro_stack.push(mchild);
                            }
                        }
                    }
                }
                _ => {
                    stack.push(child);
                }
            }
        }
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_extract_function() {
        let source = r#"
pub fn hello(name: &str) -> String {
    format!("Hello, {name}")
}
"#;
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_struct_and_impl() {
        let source = r"
pub struct Config {
    pub port: u16,
}

impl Config {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}
";
        let (symbols, _) = parse_rust_source(source, "src/config.rs").unwrap();
        let struct_sym = symbols
            .iter()
            .find(|s| s.name == "Config" && s.kind == SymbolKind::Struct)
            .unwrap();
        assert_eq!(struct_sym.kind, SymbolKind::Struct);
        let method = symbols.iter().find(|s| s.name == "new").unwrap();
        assert_eq!(method.kind, SymbolKind::Function);
    }

    #[test]
    fn test_extract_use_statements() {
        let source = r"
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let uses: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Use)
            .collect();
        assert_eq!(uses.len(), 2);
    }

    #[test]
    fn test_extract_enum_and_trait() {
        let source = r"
pub enum Color { Red, Green, Blue }
pub trait Drawable { fn draw(&self); }
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Color" && s.kind == SymbolKind::Enum)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Drawable" && s.kind == SymbolKind::Trait)
        );
    }

    #[test]
    fn test_private_function() {
        let source = r"
fn private_helper() {}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert_eq!(symbols[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_refs_type_usage() {
        let source = r"
pub struct Config { pub port: u16 }

pub fn create_config() -> Config {
    Config { port: 8080 }
}
";
        let known: std::collections::HashSet<String> = ["Config", "create_config"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let config_ref = refs.iter().find(|r| r.target_name == "Config");
        assert!(config_ref.is_some(), "should find Config reference");
        assert_eq!(config_ref.unwrap().source_name, "create_config");
    }

    #[test]
    fn test_extract_refs_struct_field_type() {
        // Regression: a type used only as a struct field (never in a fn signature
        // or body) was invisible to `impact`, because `collect_refs` did not walk
        // struct field type positions.
        let source = r"
pub struct LiveQuoteUpdate { pub px: f64 }

pub struct Book {
    pub last: Option<LiveQuoteUpdate>,
}
";
        let known: std::collections::HashSet<String> = ["LiveQuoteUpdate", "Book"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let field_ref = refs
            .iter()
            .find(|r| r.target_name == "LiveQuoteUpdate" && r.source_name == "Book");
        assert!(
            field_ref.is_some(),
            "struct field type should produce a TypeRef from Book -> LiveQuoteUpdate: {refs:?}"
        );
        assert!(matches!(field_ref.unwrap().kind, RefKind::TypeRef));
    }

    #[test]
    fn test_extract_refs_tuple_struct_field_type() {
        let source = r"
pub struct Inner { pub x: u32 }

pub struct Wrapper(pub Inner);
";
        let known: std::collections::HashSet<String> = ["Inner", "Wrapper"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let field_ref = refs
            .iter()
            .find(|r| r.target_name == "Inner" && r.source_name == "Wrapper");
        assert!(
            field_ref.is_some(),
            "tuple struct field type should produce a TypeRef from Wrapper -> Inner: {refs:?}"
        );
    }

    #[test]
    fn test_extract_refs_enum_variant_type() {
        let source = r"
pub struct Payload { pub n: u32 }

pub enum Event {
    Tick,
    Data(Payload),
}
";
        let known: std::collections::HashSet<String> = ["Payload", "Event"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let variant_ref = refs
            .iter()
            .find(|r| r.target_name == "Payload" && r.source_name == "Event");
        assert!(
            variant_ref.is_some(),
            "enum variant payload type should produce a TypeRef from Event -> Payload: {refs:?}"
        );
    }

    #[test]
    fn test_extract_refs_struct_field_no_self_ref() {
        // A struct referencing itself recursively (via a Box) should not
        // produce a Foo -> Foo self-reference.
        let source = r"
pub struct Foo {
    pub next: Option<Box<Foo>>,
}
";
        let known: std::collections::HashSet<String> =
            ["Foo"].iter().map(|s| (*s).to_string()).collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let self_ref = refs
            .iter()
            .find(|r| r.target_name == "Foo" && r.source_name == "Foo");
        assert!(
            self_ref.is_none(),
            "struct should not produce a self-reference through its own field: {refs:?}"
        );
    }

    #[test]
    fn test_extract_refs_function_call() {
        let source = r"
fn helper() -> i32 { 42 }

pub fn caller() -> i32 {
    helper()
}
";
        let known: std::collections::HashSet<String> = ["helper", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let helper_ref = refs.iter().find(|r| r.target_name == "helper");
        assert!(helper_ref.is_some(), "should find helper call");
        assert_eq!(helper_ref.unwrap().source_name, "caller");
    }

    #[test]
    fn test_extract_refs_method_call_on_parameter() {
        let source = r"
pub fn do_query() -> Vec<String> { vec![] }

pub fn caller(db: &Database) {
    let results = db.do_query();
}
";
        let known: std::collections::HashSet<String> = ["do_query", "caller", "Database"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let method_ref = refs
            .iter()
            .find(|r| r.target_name == "do_query" && r.source_name == "caller");
        assert!(
            method_ref.is_some(),
            "should detect db.do_query() as a ref to do_query: {refs:?}"
        );
    }

    #[test]
    fn test_extract_refs_no_self_ref() {
        let source = r"
pub fn standalone() -> i32 { 42 }
";
        let known: std::collections::HashSet<String> =
            ["standalone"].iter().map(|s| (*s).to_string()).collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        assert!(refs.is_empty(), "should not create self-references");
    }

    #[test]
    fn test_extract_doc_comments() {
        let source = r"
/// First line of docs
/// Second line of docs
pub fn documented() {}

pub fn undocumented() {}

/// Doc comment here
#[derive(Debug)]
pub struct Annotated;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();

        let documented = symbols.iter().find(|s| s.name == "documented").unwrap();
        let doc = documented.doc_comment.as_deref().unwrap();
        assert!(doc.contains("First line of docs"));
        assert!(doc.contains("Second line of docs"));
        assert_eq!(doc.lines().count(), 2);

        let undocumented = symbols.iter().find(|s| s.name == "undocumented").unwrap();
        assert!(undocumented.doc_comment.is_none());

        let annotated = symbols.iter().find(|s| s.name == "Annotated").unwrap();
        let doc = annotated.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Doc comment here"));
    }

    #[test]
    fn test_extract_body() {
        let source = r#"
pub fn greet(name: &str) -> String {
    format!("Hello, {name}")
}
"#;
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "greet").unwrap();
        let body = sym.body.as_deref().unwrap();
        assert!(body.contains("pub fn greet"));
        assert!(body.contains("format!"));
    }

    #[test]
    fn test_body_truncation() {
        use std::fmt::Write;
        let mut source = String::from("pub fn long_fn() {\n");
        for i in 0..110 {
            let _ = writeln!(source, "    let x{i} = {i};");
        }
        source.push_str("}\n");

        let (symbols, _) = parse_rust_source(&source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "long_fn").unwrap();
        let body = sym.body.as_deref().unwrap();
        assert!(
            body.contains("// ...") && body.contains("lines omitted"),
            "Should have omission marker"
        );
        // Head (50 lines) + marker (1 line) + tail (10 lines) = 61 lines
        assert!(body.lines().count() <= 62, "Should be truncated");
    }

    #[test]
    fn test_body_truncation_preserves_tail() {
        use std::fmt::Write;
        let mut source = String::from("pub fn big() {\n");
        for i in 0..118 {
            let _ = writeln!(source, "    let x{i} = {i};");
        }
        source.push_str("    x117\n}\n");

        let (symbols, _) = parse_rust_source(&source, "test.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "big").unwrap();
        let body = sym.body.as_deref().unwrap();
        assert!(body.contains("pub fn big()"), "Should start with signature");
        assert!(
            body.contains("x117"),
            "Should preserve tail with return value, got last 200 chars:\n{}",
            &body[body.len().saturating_sub(200)..]
        );
        assert!(
            body.contains("// ...") && body.contains("lines omitted"),
            "Should have structured omission marker, got:\n{body}"
        );
    }

    #[test]
    fn test_extract_struct_fields() {
        let source = r"
pub struct Config {
    pub port: u16,
    pub host: String,
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Config").unwrap();
        let details = sym.details.as_deref().unwrap();
        assert!(details.contains("pub port: u16"));
        assert!(details.contains("pub host: String"));
        assert_eq!(details.lines().count(), 2);
    }

    #[test]
    fn test_extract_enum_variants() {
        let source = r"
pub enum Color {
    Red,
    Green(u8),
    Blue { intensity: f32 },
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Color").unwrap();
        let details = sym.details.as_deref().unwrap();
        assert!(details.contains("Red"));
        assert!(details.contains("Green(u8)"));
        assert!(details.contains("Blue { intensity: f32 }"));
    }

    #[test]
    fn test_enum_variants_as_symbols() {
        let source = r"
pub enum Color {
    Red,
    Green(u8),
    Blue { intensity: f32 },
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let variants: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::EnumVariant)
            .collect();
        assert_eq!(variants.len(), 3, "should extract 3 variants");

        let red = variants.iter().find(|v| v.name == "Red").unwrap();
        assert_eq!(red.impl_type.as_deref(), Some("Color"));
        assert_eq!(red.visibility, Visibility::Public);

        let green = variants.iter().find(|v| v.name == "Green").unwrap();
        assert!(green.signature.contains("Green(u8)"));
        assert_eq!(green.impl_type.as_deref(), Some("Color"));

        let blue = variants.iter().find(|v| v.name == "Blue").unwrap();
        assert!(blue.signature.contains("Blue"));
        assert_eq!(blue.impl_type.as_deref(), Some("Color"));
    }

    #[test]
    fn test_extract_trait_methods() {
        let source = r"
pub trait Drawable {
    fn draw(&self);
    fn resize(&mut self, width: u32, height: u32);
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Drawable").unwrap();
        let details = sym.details.as_deref().unwrap();
        assert!(details.contains("fn draw(&self)"));
        assert!(details.contains("fn resize(&mut self, width: u32, height: u32)"));
    }

    #[test]
    fn test_unit_struct_no_details() {
        let source = r"
pub struct Empty;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Empty").unwrap();
        assert!(sym.details.is_none());
    }

    #[test]
    fn test_extract_trait_impl() {
        let source = r"
pub trait Drawable {
    fn draw(&self);
}

pub struct Circle;

impl Drawable for Circle {
    fn draw(&self) {}
}
";
        let (_, trait_impls) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert_eq!(trait_impls.len(), 1);
        assert_eq!(trait_impls[0].trait_name, "Drawable");
        assert_eq!(trait_impls[0].type_name, "Circle");
        assert_eq!(trait_impls[0].file_path, "src/lib.rs");
    }

    #[test]
    fn test_inherent_impl_not_trait_impl() {
        let source = r"
pub struct Foo;

impl Foo {
    pub fn new() -> Self { Self }
}
";
        let (_, trait_impls) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(trait_impls.is_empty());
    }

    #[test]
    fn test_extract_refs_filters_noisy_names() {
        let source = r"
pub struct Config {
    pub port: u16,
}

impl Config {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}

pub fn caller() -> Config {
    Config::new(8080)
}
";
        let known: std::collections::HashSet<String> = ["Config", "new", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        assert!(
            refs.iter().any(|r| r.target_name == "Config"),
            "should find Config as a target"
        );
        assert!(
            refs.iter().any(|r| r.target_name == "new"),
            "should find 'new' as a target (no longer filtered)"
        );
    }

    #[test]
    fn test_extract_const() {
        let source = r"
/// Maximum retry count.
pub const MAX_RETRIES: u32 = 3;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "MAX_RETRIES");
        assert!(sym.is_some(), "should extract const");
        let sym = sym.unwrap();
        assert_eq!(sym.kind, SymbolKind::Const);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(sym.doc_comment.as_deref().unwrap().contains("Maximum"));
    }

    #[test]
    fn test_extract_static() {
        let source = r"
pub static GLOBAL_COUNT: u64 = 0;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "GLOBAL_COUNT" && s.kind == SymbolKind::Static)
        );
    }

    #[test]
    fn test_extract_type_alias() {
        let source = r"
pub type Result<T> = std::result::Result<T, MyError>;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Result" && s.kind == SymbolKind::TypeAlias)
        );
    }

    #[test]
    fn test_extract_macro_rules() {
        let source = r"
/// Helper macro for creating responses.
macro_rules! response {
    ($code:expr, $body:expr) => {
        Response::new($code, $body)
    };
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "response" && s.kind == SymbolKind::Macro)
        );
    }

    #[test]
    fn test_generic_trait_impl() {
        let source = r#"
use std::fmt;

pub struct MyType;

impl fmt::Display for MyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MyType")
    }
}
"#;
        let (_, trait_impls) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert_eq!(trait_impls.len(), 1);
        assert_eq!(trait_impls[0].type_name, "MyType");
        // scoped_type_identifier gives full path
        assert!(
            trait_impls[0].trait_name.contains("Display"),
            "trait_name was: {}",
            trait_impls[0].trait_name
        );
    }

    #[test]
    fn test_extract_derives() {
        let source = r"
#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub port: u16,
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Config").unwrap();
        let attrs = sym.attributes.as_ref().unwrap();
        assert!(attrs.contains("derive(Debug, Clone, Serialize)"));
    }

    #[test]
    fn test_extract_serde_attribute() {
        let source = r#"
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub port: u16,
}
"#;
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Config").unwrap();
        let attrs = sym.attributes.as_ref().unwrap();
        assert!(attrs.contains("derive(Serialize)"));
        assert!(attrs.contains("serde(rename_all"));
    }

    #[test]
    fn test_no_attributes() {
        let source = r"
pub struct Plain {
    pub x: i32,
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Plain").unwrap();
        assert!(sym.attributes.is_none());
    }

    // ── 1. Function Signature Fidelity ──

    #[test]
    fn test_async_function_signature() {
        let source = r"
pub async fn fetch(url: &str) -> Result<String> {
    todo!()
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "fetch").unwrap();
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(
            sym.signature.contains("async fn fetch"),
            "signature was: {}",
            sym.signature
        );
    }

    #[test]
    fn test_generic_function_signature() {
        let source = r"
pub fn convert<T: Into<String>>(value: T) -> String {
    value.into()
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "convert").unwrap();
        assert!(
            sym.signature.contains("<T: Into<String>>"),
            "signature was: {}",
            sym.signature
        );
    }

    #[test]
    fn test_lifetime_function_signature() {
        let source = r"
pub fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "longest").unwrap();
        assert!(
            sym.signature.contains("<'a>"),
            "signature was: {}",
            sym.signature
        );
        assert!(
            sym.signature.contains("&'a str"),
            "signature was: {}",
            sym.signature
        );
    }

    #[test]
    fn test_where_clause_in_signature() {
        let source = r"
pub fn process<T>(items: Vec<T>) -> String
where
    T: std::fmt::Display,
{
    String::new()
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "process").unwrap();
        assert!(
            sym.signature.contains("where"),
            "signature should contain where clause: {}",
            sym.signature
        );
        assert!(
            sym.signature.contains("T: std::fmt::Display"),
            "signature should contain trait bound: {}",
            sym.signature
        );
        assert!(
            !sym.signature.contains("String::new"),
            "signature should not contain body: {}",
            sym.signature
        );
    }

    #[test]
    fn test_unsafe_and_extern_function() {
        let source = r#"
pub unsafe fn dangerous(ptr: *mut u8) {}
extern "C" fn callback(code: i32) {}
"#;
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();

        let dangerous = symbols.iter().find(|s| s.name == "dangerous").unwrap();
        assert!(
            dangerous.signature.contains("unsafe fn"),
            "signature was: {}",
            dangerous.signature
        );
        assert_eq!(dangerous.visibility, Visibility::Public);

        let callback = symbols.iter().find(|s| s.name == "callback").unwrap();
        assert!(
            callback.signature.contains("extern \"C\" fn"),
            "signature was: {}",
            callback.signature
        );
        assert_eq!(callback.visibility, Visibility::Private);
    }

    // ── 2. Generic & Complex Types ──

    #[test]
    fn test_generic_struct_details() {
        let source = r"
pub struct Wrapper<T: Clone> {
    pub inner: T,
    pub count: usize,
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "Wrapper" && s.kind == SymbolKind::Struct)
            .unwrap();
        assert!(
            sym.signature.contains("<T: Clone>"),
            "signature was: {}",
            sym.signature
        );
        let details = sym.details.as_deref().unwrap();
        assert!(details.contains("pub inner: T"));
        assert!(details.contains("pub count: usize"));
    }

    #[test]
    fn test_const_generic_struct() {
        let source = r"
pub struct FixedArray<const N: usize> {
    pub data: [u8; N],
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "FixedArray" && s.kind == SymbolKind::Struct)
            .unwrap();
        assert!(
            sym.signature.contains("<const N: usize>"),
            "signature was: {}",
            sym.signature
        );
    }

    #[test]
    fn test_generic_trait_impl_strips_params() {
        let source = r"
pub struct Wrapper<T>(T);

impl<T: Clone> From<Vec<T>> for Wrapper<T> {
    fn from(v: Vec<T>) -> Self {
        Self(v.into_iter().next().unwrap())
    }
}
";
        let (_, trait_impls) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert_eq!(trait_impls.len(), 1);
        assert_eq!(
            trait_impls[0].trait_name, "From",
            "should strip generic params from trait name"
        );
        assert_eq!(
            trait_impls[0].type_name, "Wrapper",
            "should strip generic params from type name"
        );
    }

    #[test]
    fn test_tuple_struct_field_details() {
        let source = r"
pub struct Pair(pub u32, pub String);
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "Pair" && s.kind == SymbolKind::Struct)
            .unwrap();
        let details = sym.details.as_deref().unwrap();
        assert!(
            details.contains("pub u32"),
            "tuple struct should list field types: {details}"
        );
        assert!(
            details.contains("pub String"),
            "tuple struct should list field types: {details}"
        );
    }

    // ── 3. Trait Extraction ──

    #[test]
    fn test_trait_associated_type_in_details() {
        let source = r"
pub trait Iterator {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "Iterator" && s.kind == SymbolKind::Trait)
            .unwrap();
        let details = sym.details.as_deref().unwrap();
        assert!(
            details.contains("fn next"),
            "details should have fn next: {details}"
        );
        assert!(
            details.contains("type Item"),
            "details should contain associated type: {details}"
        );
    }

    #[test]
    fn test_trait_default_method_body() {
        let source = r#"
pub trait Greeting {
    fn name(&self) -> &str;
    fn greet(&self) -> String {
        format!("Hello, {}!", self.name())
    }
}
"#;
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "Greeting" && s.kind == SymbolKind::Trait)
            .unwrap();
        let details = sym.details.as_deref().unwrap();
        assert!(details.contains("fn name(&self)"));
        assert!(details.contains("fn greet(&self)"));
        assert_eq!(details.lines().count(), 2);
    }

    #[test]
    fn test_empty_trait_no_details() {
        let source = r"
pub trait Marker {}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "Marker" && s.kind == SymbolKind::Trait)
            .unwrap();
        assert!(sym.details.is_none(), "empty trait should have no details");
    }

    // ── 4. Visibility Edge Cases ──

    #[test]
    fn test_pub_super_classified_as_restricted() {
        let source = r"
pub(super) fn parent_visible() {}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "parent_visible").unwrap();
        assert_eq!(sym.visibility, Visibility::Restricted);
    }

    #[test]
    fn test_pub_in_path_classified_as_restricted() {
        let source = r"
pub(in crate::foo) fn scoped_visible() {}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "scoped_visible").unwrap();
        assert_eq!(sym.visibility, Visibility::Restricted);
    }

    #[test]
    fn test_pub_crate_function() {
        let source = r"
pub(crate) fn internal_helper() -> i32 { 42 }
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "internal_helper")
            .unwrap();
        assert_eq!(sym.visibility, Visibility::PublicCrate);
        assert!(
            sym.signature.contains("pub(crate)"),
            "signature was: {}",
            sym.signature
        );
    }

    // ── 5. Reference Extraction ──

    #[test]
    fn test_refs_from_impl_method() {
        let source = r"
fn helper() -> i32 { 42 }

pub struct Foo;

impl Foo {
    fn method(&self) -> i32 {
        helper()
    }
}
";
        let known: std::collections::HashSet<String> = ["helper", "method", "Foo"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let helper_ref = refs
            .iter()
            .find(|r| r.target_name == "helper" && r.source_name == "method");
        assert!(
            helper_ref.is_some(),
            "should find helper call from impl method, refs: {refs:?}"
        );
    }

    #[test]
    fn test_refs_inside_closure() {
        let source = r"
fn target() -> i32 { 42 }

fn caller() {
    let f = |x: i32| target() + x;
}
";
        let known: std::collections::HashSet<String> = ["target", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let target_ref = refs
            .iter()
            .find(|r| r.target_name == "target" && r.source_name == "caller");
        assert!(
            target_ref.is_some(),
            "should find target call inside closure, refs: {refs:?}"
        );
    }

    #[test]
    fn test_refs_type_in_function_body() {
        let source = r"
pub struct Config {
    pub port: u16,
}

fn make() -> Config {
    Config { port: 8080 }
}
";
        let known: std::collections::HashSet<String> = ["Config", "make"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let config_ref = refs
            .iter()
            .find(|r| r.target_name == "Config" && r.source_name == "make");
        assert!(
            config_ref.is_some(),
            "should find Config type ref in function body, refs: {refs:?}"
        );
    }

    #[test]
    fn test_refs_noisy_filtered_in_chain() {
        let source = r"
fn setup() -> String { String::new() }

fn caller() -> String {
    setup().to_string()
}
";
        let known: std::collections::HashSet<String> = ["setup", "caller", "to_string"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        assert!(
            refs.iter().any(|r| r.target_name == "setup"),
            "should find setup ref"
        );
        assert!(
            !refs.iter().any(|r| r.target_name == "to_string"),
            "to_string should be filtered as noisy"
        );
    }

    #[test]
    fn test_refs_deduplicated() {
        let source = r"
fn target() -> i32 { 1 }

fn caller() -> i32 {
    target() + target() + target()
}
";
        let known: std::collections::HashSet<String> = ["target", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let target_refs: Vec<_> = refs.iter().filter(|r| r.target_name == "target").collect();
        assert_eq!(
            target_refs.len(),
            1,
            "should deduplicate refs to same target, got: {target_refs:?}"
        );
    }

    #[test]
    fn test_locals_excluded_from_refs() {
        let source = r"
pub struct Config {}

fn builder() {
    let Config = 42;
    Config + 1
}
";
        let known: std::collections::HashSet<String> = ["Config", "builder"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        assert!(
            !refs
                .iter()
                .any(|r| r.source_name == "builder" && r.target_name == "Config"),
            "local `let Config` should shadow the struct, refs: {refs:?}"
        );
    }

    #[test]
    fn test_param_excluded_from_refs() {
        let source = r"
pub fn target() {}

fn caller(target: i32) -> i32 {
    target + 1
}
";
        let known: std::collections::HashSet<String> = ["target", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        assert!(
            !refs
                .iter()
                .any(|r| r.source_name == "caller" && r.target_name == "target"),
            "param `target` should shadow the function, refs: {refs:?}"
        );
    }

    // ── 6. Edge Cases & Robustness ──

    #[test]
    fn test_empty_enum_no_details() {
        let source = r"
pub enum Never {}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "Never" && s.kind == SymbolKind::Enum)
            .unwrap();
        assert!(sym.details.is_none(), "empty enum should have no details");
    }

    #[test]
    fn test_inline_mod_extracts_inner_symbols() {
        let source = r"
pub mod outer {
    pub fn inner_fn() {}
    pub struct InnerStruct;
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "outer" && s.kind == SymbolKind::Mod),
            "should extract mod symbol"
        );
        assert!(
            symbols.iter().any(|s| s.name == "inner_fn"),
            "should extract inner_fn from inline mod body"
        );
        assert!(
            symbols.iter().any(|s| s.name == "InnerStruct"),
            "should extract InnerStruct from inline mod body"
        );
    }

    #[test]
    fn test_pub_use_reexport() {
        let source = r"
pub use crate::inner::Symbol;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.kind == SymbolKind::Use).unwrap();
        assert_eq!(sym.visibility, Visibility::Public);
    }

    #[test]
    fn test_multiple_impl_blocks_same_type() {
        let source = r"
pub struct Widget {
    pub width: u32,
}

impl Widget {
    pub fn area(&self) -> u32 { self.width * self.width }
}

impl Widget {
    pub fn perimeter(&self) -> u32 { self.width * 4 }
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let structs: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        let impls: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Impl)
            .collect();
        let fns: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(structs.len(), 1, "1 struct");
        assert_eq!(impls.len(), 2, "2 impl blocks");
        assert_eq!(fns.len(), 2, "2 methods");
        assert_eq!(symbols.len(), 5, "5 total symbols");
    }

    #[test]
    fn test_complex_attributes() {
        let source = r"
#[cfg(test)]
#[tokio::test]
#[instrument(skip(self))]
pub fn instrumented_test() {}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "instrumented_test")
            .unwrap();
        let attrs = sym.attributes.as_deref().unwrap();
        assert!(attrs.contains("cfg(test)"), "attrs: {attrs}");
        assert!(attrs.contains("tokio::test"), "attrs: {attrs}");
        assert!(attrs.contains("instrument(skip(self))"), "attrs: {attrs}");
        // Comma-separated
        assert_eq!(
            attrs.matches(", ").count(),
            2,
            "should have 3 attrs joined by commas: {attrs}"
        );
    }

    #[test]
    fn test_doc_comment_with_code_block() {
        let source = r"
/// Creates a new instance.
///
/// # Examples
///
/// ```
/// let x = MyType::new();
/// assert!(x.is_valid());
/// ```
pub fn documented_with_code() {}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols
            .iter()
            .find(|s| s.name == "documented_with_code")
            .unwrap();
        let doc = sym.doc_comment.as_deref().unwrap();
        assert!(doc.contains("# Examples"), "doc: {doc}");
        assert!(doc.contains("```"), "doc should preserve backticks: {doc}");
        assert!(
            doc.contains("let x = MyType::new()"),
            "doc should preserve code content: {doc}"
        );
    }

    // ── 7. Import Map Extraction ──

    #[test]
    fn test_extract_import_map_direct() {
        let source = "use crate::config::Config;\n";
        let map = extract_import_map_from_source(source).unwrap();
        assert_eq!(map.len(), 1);
        let info = map.get("Config").unwrap();
        assert_eq!(info.qualified_path, "crate::config::Config");
    }

    #[test]
    fn test_extract_import_map_alias() {
        let source = "use anyhow::Result as AnyResult;\n";
        let map = extract_import_map_from_source(source).unwrap();
        assert_eq!(map.len(), 1);
        let info = map.get("AnyResult").unwrap();
        assert_eq!(info.qualified_path, "anyhow::Result");
    }

    #[test]
    fn test_extract_import_map_nested() {
        let source = "use std::collections::{HashMap, HashSet};\n";
        let map = extract_import_map_from_source(source).unwrap();
        assert_eq!(map.len(), 2);
        let hm = map.get("HashMap").unwrap();
        assert_eq!(hm.qualified_path, "std::collections::HashMap");
        let hs = map.get("HashSet").unwrap();
        assert_eq!(hs.qualified_path, "std::collections::HashSet");
    }

    #[test]
    fn test_extract_import_map_skips_glob() {
        let source = "use std::collections::*;\n";
        let map = extract_import_map_from_source(source).unwrap();
        assert!(
            map.is_empty(),
            "glob imports should be skipped, got: {map:?}"
        );
    }

    #[test]
    fn test_qualified_path_to_file_resolves() {
        let result = qualified_path_to_file("crate::config::Config");
        assert_eq!(result, Some("src/config.rs".to_string()));
    }

    #[test]
    fn test_qualified_path_to_file_nested() {
        let result = qualified_path_to_file("crate::server::tools::query");
        assert_eq!(result, Some("src/server/tools.rs".to_string()));
    }

    #[test]
    fn test_qualified_path_to_file_non_crate() {
        let result = qualified_path_to_file("anyhow::Result");
        assert!(result.is_none());
    }

    #[test]
    fn test_qualified_path_to_file_cross_crate() {
        let mut crate_map = std::collections::HashMap::new();
        crate_map.insert("shared".to_string(), "shared".to_string());
        crate_map.insert("core_lib".to_string(), "libs/core".to_string());

        let first = |p: &str| {
            qualified_path_to_files_with_crates(p, &crate_map)
                .into_iter()
                .next()
        };

        // crate:: still works with crate_map present
        assert_eq!(
            first("crate::config::Config"),
            Some("src/config.rs".to_string()),
        );

        // workspace crate, top-level symbol -> lib.rs
        assert_eq!(
            first("shared::Config"),
            Some("shared/src/lib.rs".to_string()),
        );

        // workspace crate, nested module
        assert_eq!(
            first("shared::config::Config"),
            Some("shared/src/config.rs".to_string()),
        );

        // workspace crate with non-trivial path
        assert_eq!(
            first("core_lib::models::User"),
            Some("libs/core/src/models.rs".to_string()),
        );

        // unknown crate returns empty
        assert!(qualified_path_to_files_with_crates("unknown::Thing", &crate_map).is_empty());

        // single segment returns empty
        assert!(qualified_path_to_files_with_crates("Config", &crate_map).is_empty());
    }

    #[test]
    fn test_qualified_path_returns_mod_rs_variant() {
        let crate_map = std::collections::HashMap::<String, String>::new();
        let candidates =
            qualified_path_to_files_with_crates("crate::server::tools::Config", &crate_map);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0], "src/server/tools.rs");
        assert_eq!(candidates[1], "src/server/tools/mod.rs");
    }

    #[test]
    fn test_refs_include_target_file_from_import() {
        let source = r"
use crate::config::Config;

pub fn make() -> Config {
    Config { port: 8080 }
}
";
        let known: std::collections::HashSet<String> = ["Config", "make"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let config_ref = refs.iter().find(|r| r.target_name == "Config").unwrap();
        assert_eq!(
            config_ref.target_file.as_deref(),
            Some("src/config.rs"),
            "should resolve target_file from import map"
        );
    }

    #[test]
    fn test_refs_same_file_target_file_without_import() {
        let source = r"
pub struct Config { pub port: u16 }

pub fn make() -> Config {
    Config { port: 8080 }
}
";
        let known: std::collections::HashSet<String> = ["Config", "make"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let config_ref = refs.iter().find(|r| r.target_name == "Config").unwrap();
        assert_eq!(
            config_ref.target_file.as_deref(),
            Some("src/lib.rs"),
            "same-file symbol should have target_file set to source file"
        );
    }

    #[test]
    fn test_symbol_impl_type() {
        let source = r"
pub struct MyStruct;
impl MyStruct {
    pub fn method(&self) {}
}
pub fn free_function() {}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let method = symbols.iter().find(|s| s.name == "method").unwrap();
        assert_eq!(method.impl_type.as_deref(), Some("MyStruct"));
        let free_fn = symbols.iter().find(|s| s.name == "free_function").unwrap();
        assert!(free_fn.impl_type.is_none());
    }

    #[test]
    fn test_refs_self_method_has_target_context() {
        let source = r"
pub struct MyStruct;

impl MyStruct {
    pub fn do_work(&self) {
        self.helper()
    }

    pub fn helper(&self) {}
}
";
        let known: std::collections::HashSet<String> = ["do_work", "helper", "MyStruct"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let helper_ref = refs
            .iter()
            .find(|r| r.target_name == "helper" && r.source_name == "do_work");
        assert!(
            helper_ref.is_some(),
            "should find helper call from do_work, refs: {refs:?}"
        );
        assert_eq!(
            helper_ref.unwrap().target_context.as_deref(),
            Some("MyStruct"),
            "self.helper() should have target_context = MyStruct"
        );
    }

    #[test]
    fn test_extract_union() {
        let source = r"
pub union MyUnion {
    pub i: i32,
    pub f: f32,
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let u = symbols.iter().find(|s| s.name == "MyUnion").unwrap();
        assert_eq!(u.kind, SymbolKind::Union);
        assert!(u.details.as_ref().is_some_and(|d| d.contains("i: i32")));
    }

    #[test]
    fn test_extract_extern_block_functions() {
        let source = "extern \"C\" {\n    \
                       pub fn c_function(x: i32) -> i32;\n    \
                       pub fn another_c_fn();\n\
                       }\n";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let c_fn = symbols.iter().find(|s| s.name == "c_function");
        assert!(
            c_fn.is_some(),
            "extern C functions should be extracted: {symbols:?}"
        );
        let another = symbols.iter().find(|s| s.name == "another_c_fn");
        assert!(
            another.is_some(),
            "second extern C function should be extracted: {symbols:?}"
        );
    }

    #[test]
    fn test_extract_const_fn() {
        let source = "pub const fn compute(x: u32) -> u32 { x + 1 }\n";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let f = symbols.iter().find(|s| s.name == "compute").unwrap();
        assert_eq!(f.kind, SymbolKind::Function);
        assert!(f.signature.contains("const fn"));
    }

    #[test]
    fn test_multiline_signature_captured() {
        let source = r"
pub fn handle_query(
    db: &Database,
    query: &str,
    scope: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    todo!()
}
";
        let (symbols, _) = parse_rust_source(source, "test.rs").unwrap();
        let func = symbols.iter().find(|s| s.name == "handle_query").unwrap();
        assert!(
            func.signature.contains("-> Result<String"),
            "return type in sig: {}",
            func.signature
        );
        assert!(
            func.signature.contains("&Database"),
            "param type in sig: {}",
            func.signature
        );
        assert!(
            !func.signature.contains("todo!"),
            "body not in sig: {}",
            func.signature
        );
    }

    #[test]
    fn test_same_file_refs_have_target_file() {
        let source = r"
fn helper() -> i32 { 42 }

pub fn caller() -> i32 {
    helper()
}
";
        let known_symbols: std::collections::HashSet<String> = ["helper", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let crate_map = std::collections::HashMap::new();
        let refs = extract_refs(source, "src/lib.rs", &known_symbols, &crate_map).unwrap();
        let caller_to_helper = refs
            .iter()
            .find(|r| r.source_name == "caller" && r.target_name == "helper");
        assert!(
            caller_to_helper.is_some(),
            "should find ref from caller to helper"
        );
        let r = caller_to_helper.unwrap();
        assert_eq!(
            r.target_file.as_deref(),
            Some("src/lib.rs"),
            "same-file ref should have target_file set to source file"
        );
    }

    // ── 8. Local Type Inference & Method Resolution ──

    #[test]
    fn test_refs_param_type_method_has_context() {
        let source = r"
pub struct Database;

impl Database {
    pub fn search(&self) -> Vec<String> { vec![] }
}

pub fn caller(db: &Database) {
    db.search();
}
";
        let known: std::collections::HashSet<String> = ["caller", "search", "Database"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let search_ref = refs
            .iter()
            .find(|r| r.target_name == "search" && r.source_name == "caller");
        assert!(
            search_ref.is_some(),
            "should find db.search() call, refs: {refs:?}"
        );
        assert_eq!(
            search_ref.unwrap().target_context.as_deref(),
            Some("Database"),
            "db: &Database → db.search() should have target_context = Database"
        );
    }

    #[test]
    fn test_refs_param_mut_ref_type_method_has_context() {
        let source = r"
pub struct Config;

impl Config {
    pub fn reload(&mut self) {}
}

pub fn updater(config: &mut Config) {
    config.reload();
}
";
        let known: std::collections::HashSet<String> = ["updater", "reload", "Config"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let reload_ref = refs
            .iter()
            .find(|r| r.target_name == "reload" && r.source_name == "updater");
        assert!(
            reload_ref.is_some(),
            "should find config.reload() call, refs: {refs:?}"
        );
        assert_eq!(
            reload_ref.unwrap().target_context.as_deref(),
            Some("Config"),
            "&mut Config → config.reload() should have target_context = Config"
        );
    }

    #[test]
    fn test_refs_let_constructor_infers_type() {
        let source = r"
pub struct Database;

impl Database {
    pub fn open() -> Self { Database }
    pub fn query(&self) {}
}

pub fn caller() {
    let db = Database::open();
    db.query();
}
";
        let known: std::collections::HashSet<String> = ["caller", "open", "query", "Database"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let query_ref = refs
            .iter()
            .find(|r| r.target_name == "query" && r.source_name == "caller");
        assert!(
            query_ref.is_some(),
            "should find db.query() call, refs: {refs:?}"
        );
        assert_eq!(
            query_ref.unwrap().target_context.as_deref(),
            Some("Database"),
            "let db = Database::open() → db.query() should resolve to Database"
        );
    }

    #[test]
    fn test_refs_let_struct_literal_infers_type() {
        let source = r"
pub struct Config {
    pub port: u16,
}

impl Config {
    pub fn validate(&self) -> bool { true }
}

pub fn caller() {
    let config = Config { port: 8080 };
    config.validate();
}
";
        let known: std::collections::HashSet<String> = ["caller", "validate", "Config"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let validate_ref = refs
            .iter()
            .find(|r| r.target_name == "validate" && r.source_name == "caller");
        assert!(
            validate_ref.is_some(),
            "should find config.validate() call, refs: {refs:?}"
        );
        assert_eq!(
            validate_ref.unwrap().target_context.as_deref(),
            Some("Config"),
            "let config = Config {{ ... }} → config.validate() should resolve to Config"
        );
    }

    #[test]
    fn test_refs_let_with_try_operator_infers_type() {
        let source = r"
pub struct Database;

impl Database {
    pub fn open() -> Result<Self, String> { Ok(Database) }
    pub fn query(&self) {}
}

pub fn caller() -> Result<(), String> {
    let db = Database::open()?;
    db.query();
    Ok(())
}
";
        let known: std::collections::HashSet<String> = ["caller", "open", "query", "Database"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let query_ref = refs
            .iter()
            .find(|r| r.target_name == "query" && r.source_name == "caller");
        assert!(
            query_ref.is_some(),
            "should find db.query() after try operator, refs: {refs:?}"
        );
        assert_eq!(
            query_ref.unwrap().target_context.as_deref(),
            Some("Database"),
            "let db = Database::open()? → db.query() should resolve to Database"
        );
    }

    #[test]
    fn test_refs_let_with_type_annotation() {
        let source = r"
pub struct Server;

impl Server {
    pub fn start(&self) {}
}

pub fn caller() {
    let server: Server = todo!();
    server.start();
}
";
        let known: std::collections::HashSet<String> = ["caller", "start", "Server"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let start_ref = refs
            .iter()
            .find(|r| r.target_name == "start" && r.source_name == "caller");
        assert!(
            start_ref.is_some(),
            "should find server.start() call, refs: {refs:?}"
        );
        assert_eq!(
            start_ref.unwrap().target_context.as_deref(),
            Some("Server"),
            "let server: Server = ... → server.start() should resolve to Server"
        );
    }

    #[test]
    fn test_refs_no_context_for_unknown_variable() {
        let source = r"
pub fn search() {}

pub fn caller(x: i32) {
    x.search();
}
";
        let known: std::collections::HashSet<String> = ["caller", "search"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let search_ref = refs
            .iter()
            .find(|r| r.target_name == "search" && r.source_name == "caller");
        // i32 is a primitive, not in our type system, so no context
        if let Some(r) = search_ref {
            assert!(
                r.target_context.is_none(),
                "primitive type param should not produce target_context"
            );
        }
    }

    #[test]
    fn test_local_types_param_owned() {
        let source = r"
pub struct Config;

impl Config {
    pub fn save(&self) {}
}

pub fn caller(config: Config) {
    config.save();
}
";
        let known: std::collections::HashSet<String> = ["caller", "save", "Config"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let save_ref = refs
            .iter()
            .find(|r| r.target_name == "save" && r.source_name == "caller");
        assert!(
            save_ref.is_some(),
            "should find config.save() call, refs: {refs:?}"
        );
        assert_eq!(
            save_ref.unwrap().target_context.as_deref(),
            Some("Config"),
            "config: Config → config.save() should resolve to Config"
        );
    }

    #[test]
    fn test_extract_refs_inside_mod_item() {
        let source = r"
pub struct Database;

impl Database {
    pub fn open_in_memory() -> Self { Database }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_creates_schema() {
        let db = Database::open_in_memory();
    }
}
";
        let known: std::collections::HashSet<String> =
            ["Database", "open_in_memory", "test_creates_schema"]
                .iter()
                .map(|s| (*s).to_string())
                .collect();
        let refs = extract_refs(
            source,
            "src/db.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        assert!(
            refs.iter().any(
                |r| r.source_name == "test_creates_schema" && r.target_name == "open_in_memory"
            ),
            "should extract refs from functions inside mod items, refs: {refs:?}"
        );
    }

    #[test]
    fn test_crate_path_refs_bypass_noisy_filter() {
        // "set" is in NOISY_SYMBOL_NAMES, but crate::status::set()
        // should still be tracked because the qualified path is unambiguous
        let source = r#"
pub fn set(msg: &str) {}
pub struct StatusGuard;

impl StatusGuard {
    pub fn new(msg: &str) -> Self { Self }
}

pub fn caller() {
    crate::status::set("hello");
    let _g = crate::status::StatusGuard::new("test");
}
"#;
        let known: std::collections::HashSet<String> = ["set", "StatusGuard", "new", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/status.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();

        let set_ref = refs
            .iter()
            .find(|r| r.target_name == "set" && r.source_name == "caller");
        assert!(
            set_ref.is_some(),
            "crate::status::set() should be tracked despite 'set' being noisy, refs: {refs:?}"
        );

        let guard_ref = refs
            .iter()
            .find(|r| r.target_name == "new" && r.source_name == "caller");
        assert!(
            guard_ref.is_some(),
            "crate::status::StatusGuard::new() should be tracked, refs: {refs:?}"
        );
        assert_eq!(
            guard_ref.unwrap().target_context.as_deref(),
            Some("StatusGuard"),
            "should extract StatusGuard as target_context"
        );

        let type_ref = refs
            .iter()
            .find(|r| r.target_name == "StatusGuard" && r.source_name == "caller");
        assert!(
            type_ref.is_some(),
            "StatusGuard should also be tracked as a TypeRef, refs: {refs:?}"
        );
    }

    #[test]
    fn test_crate_path_resolves_target_file() {
        let source = r"
pub fn caller() {
    crate::indexer::docs::pending_docs();
}
";
        let known: std::collections::HashSet<String> = ["pending_docs", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/server/mod.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();

        let pd_ref = refs
            .iter()
            .find(|r| r.target_name == "pending_docs" && r.source_name == "caller");
        assert!(
            pd_ref.is_some(),
            "crate::indexer::docs::pending_docs should be tracked, refs: {refs:?}"
        );
        assert_eq!(
            pd_ref.unwrap().target_file.as_deref(),
            Some("src/indexer/docs.rs"),
            "target_file should resolve from crate:: path"
        );
    }

    #[test]
    fn test_non_crate_scoped_identifier_descends() {
        // Non-crate:: scoped identifiers like Module::func should
        // still descend into children as before
        let source = r"
pub struct Foo;

impl Foo {
    pub fn bar() -> Self { Self }
}

pub fn caller() {
    let _ = Foo::bar();
}
";
        let known: std::collections::HashSet<String> = ["Foo", "bar", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();

        assert!(
            refs.iter()
                .any(|r| r.target_name == "bar" && r.source_name == "caller"),
            "Foo::bar() should still work via normal descend, refs: {refs:?}"
        );
    }

    #[test]
    fn test_scoped_call_has_type_context() {
        let source = r"
struct Database;
impl Database {
    fn new() -> Self { Database }
}
fn caller() {
    let db = Database::new();
}
";
        let known: std::collections::HashSet<String> = ["Database", "new", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs =
            extract_refs(source, "test.rs", &known, &std::collections::HashMap::new()).unwrap();
        let new_ref = refs
            .iter()
            .find(|r| r.source_name == "caller" && r.target_name == "new");
        assert!(
            new_ref.is_some(),
            "should find ref to new from caller, refs: {refs:?}"
        );
        assert_eq!(
            new_ref.unwrap().target_context.as_deref(),
            Some("Database"),
            "scoped call Database::new should have target_context = Database"
        );
    }

    #[test]
    fn test_external_scoped_call_filtered() {
        let source = r"
struct MyStruct;
impl MyStruct {
    fn new() -> Self { MyStruct }
}
fn caller() {
    let v = Vec::new();
}
";
        let known: std::collections::HashSet<String> = ["MyStruct", "new", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs =
            extract_refs(source, "test.rs", &known, &std::collections::HashMap::new()).unwrap();
        let new_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.source_name == "caller" && r.target_name == "new")
            .collect();
        assert!(
            new_refs.is_empty(),
            "Vec::new() should not create ref to 'new', found: {new_refs:?}"
        );
    }

    #[test]
    fn test_qualified_call_bypasses_noisy_filter() {
        // `clear` is in NOISY_SYMBOL_NAMES, but `Status::clear()` should still
        // be captured because the type qualification makes it unambiguous.
        let source = r"
pub struct Status;
impl Status {
    pub fn clear(&self) {}
}
fn caller() {
    let s = Status;
    Status::clear(&s);
}
";
        let known = ["Status", "clear", "caller"]
            .into_iter()
            .map(String::from)
            .collect::<std::collections::HashSet<_>>();
        let crate_map = std::collections::HashMap::<String, String>::new();
        let refs = extract_refs(source, "src/status.rs", &known, &crate_map).unwrap();
        let has_clear_ref = refs
            .iter()
            .any(|r| r.source_name == "caller" && r.target_name == "clear");
        assert!(
            has_clear_ref,
            "qualified call Status::clear() should bypass noisy filter, got refs: {refs:?}"
        );
    }

    #[test]
    fn test_seen_dedup_includes_context() {
        // Two different Type::new() calls in the same function should both be captured
        let source = r"
pub struct Foo;
impl Foo { pub fn new() -> Self { Self } }
pub struct Bar;
impl Bar { pub fn new() -> Self { Self } }
fn caller() {
    let _ = Foo::new();
    let _ = Bar::new();
}
";
        let known = ["Foo", "Bar", "new", "caller"]
            .into_iter()
            .map(String::from)
            .collect::<std::collections::HashSet<_>>();
        let crate_map = std::collections::HashMap::<String, String>::new();
        let refs = extract_refs(source, "src/lib.rs", &known, &crate_map).unwrap();
        let new_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.source_name == "caller" && r.target_name == "new")
            .collect();
        assert_eq!(
            new_refs.len(),
            2,
            "Both Foo::new() and Bar::new() should be captured, got: {new_refs:?}"
        );
    }

    #[test]
    fn test_refs_super_path_from_sibling_file() {
        let source = r"
pub fn handle_impact() {
    super::resolve_symbol();
}
";
        let known: std::collections::HashSet<String> = ["handle_impact", "resolve_symbol"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/server/tools/impact.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let rs_ref = refs
            .iter()
            .find(|r| r.target_name == "resolve_symbol" && r.source_name == "handle_impact");
        assert!(
            rs_ref.is_some(),
            "should find super::resolve_symbol() call, refs: {refs:?}"
        );
        assert_eq!(
            rs_ref.unwrap().target_file.as_deref(),
            Some("src/server/tools/mod.rs"),
            "super:: from impact.rs should resolve to mod.rs"
        );
    }

    #[test]
    fn test_refs_self_path_resolves_to_current_file() {
        let source = r"
pub fn helper() {}

pub fn caller() {
    self::helper();
}
";
        let known: std::collections::HashSet<String> = ["caller", "helper"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/server/tools/mod.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let helper_ref = refs
            .iter()
            .find(|r| r.target_name == "helper" && r.source_name == "caller");
        assert!(
            helper_ref.is_some(),
            "should find self::helper() call, refs: {refs:?}"
        );
        assert_eq!(
            helper_ref.unwrap().target_file.as_deref(),
            Some("src/server/tools/mod.rs"),
            "self:: should resolve to the current file"
        );
    }

    #[test]
    fn test_refs_local_var_does_not_shadow_method_call() {
        // let file_count = db.file_count()?; — the local "file_count"
        // should NOT prevent tracking the db.file_count() method call
        let source = r"
pub struct Database;

impl Database {
    pub fn file_count(&self) -> Result<i64, String> { Ok(0) }
}

pub fn caller(db: &Database) -> Result<(), String> {
    let file_count = db.file_count()?;
    let _ = file_count + 1;
    Ok(())
}
";
        let known: std::collections::HashSet<String> = ["caller", "file_count", "Database"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            source,
            "src/lib.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();
        let fc_ref = refs
            .iter()
            .find(|r| r.target_name == "file_count" && r.source_name == "caller");
        assert!(
            fc_ref.is_some(),
            "db.file_count() should be tracked even when 'file_count' is a local var, refs: {refs:?}"
        );
        assert_eq!(
            fc_ref.unwrap().target_context.as_deref(),
            Some("Database"),
            "db.file_count() should have Database context"
        );
    }

    #[test]
    fn test_super_ref_produces_high_confidence_in_store() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let mod_file_id = db.insert_file("src/tools/mod.rs", "h1").unwrap();
        let impact_file_id = db.insert_file("src/tools/impact.rs", "h2").unwrap();

        let mod_source = r"
pub fn resolve_symbol() {}
";
        let impact_source = r"
pub fn handle_impact() {
    super::resolve_symbol();
}
";
        let (mod_syms, _) = parse_rust_source(mod_source, "src/tools/mod.rs").unwrap();
        let (impact_syms, _) = parse_rust_source(impact_source, "src/tools/impact.rs").unwrap();
        crate::indexer::store::store_symbols(&db, mod_file_id, &mod_syms).unwrap();
        crate::indexer::store::store_symbols(&db, impact_file_id, &impact_syms).unwrap();

        let known: std::collections::HashSet<String> = ["resolve_symbol", "handle_impact"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            impact_source,
            "src/tools/impact.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();

        let map = db.build_symbol_id_map().unwrap();
        let count = db.store_symbol_refs_fast(&refs, &map).unwrap();
        assert!(count > 0, "should store at least one ref");

        let callers = db
            .get_callers("resolve_symbol", "src/tools/mod.rs", false, Some("high"))
            .unwrap();
        assert!(
            callers.iter().any(|c| c.name == "handle_impact"),
            "handle_impact should be a HIGH confidence caller via super::, got: {callers:?}"
        );
    }

    #[test]
    fn test_local_shadow_method_produces_ref_in_store() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let db_file_id = db.insert_file("src/db.rs", "h1").unwrap();
        let caller_file_id = db.insert_file("src/caller.rs", "h2").unwrap();

        let db_source = r"
pub struct Database;
impl Database {
    pub fn file_count(&self) -> i64 { 0 }
}
";
        let caller_source = r"
pub fn caller(db: &Database) {
    let file_count = db.file_count();
    let _ = file_count;
}
";
        let (db_syms, _) = parse_rust_source(db_source, "src/db.rs").unwrap();
        let (caller_syms, _) = parse_rust_source(caller_source, "src/caller.rs").unwrap();
        crate::indexer::store::store_symbols(&db, db_file_id, &db_syms).unwrap();
        crate::indexer::store::store_symbols(&db, caller_file_id, &caller_syms).unwrap();

        let known: std::collections::HashSet<String> = ["Database", "file_count", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(
            caller_source,
            "src/caller.rs",
            &known,
            &std::collections::HashMap::new(),
        )
        .unwrap();

        let map = db.build_symbol_id_map().unwrap();
        db.store_symbol_refs_fast(&refs, &map).unwrap();

        let callers = db
            .get_callers("file_count", "src/db.rs", false, Some("high"))
            .unwrap();
        assert!(
            callers.iter().any(|c| c.name == "caller"),
            "caller should reference Database::file_count despite local shadowing, got: {callers:?}"
        );
    }

    /// End-to-end test: index the real repo, then check that `ensure_indexed`
    /// has callees stored in the DB (not just extracted by `extract_refs`).
    #[test]
    fn test_refs_real_main_rs_ensure_indexed() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let config = crate::indexer::IndexConfig {
            repo_path: std::path::PathBuf::from("."),
        };
        crate::indexer::index_repo(&db, &config).unwrap();

        let callees = db
            .get_callees("ensure_indexed", "src/main.rs", false)
            .unwrap();
        assert!(
            callees
                .iter()
                .any(|c| c.name == "open" && c.impl_type.as_deref() == Some("Database")),
            "ensure_indexed should have Database::open as callee, got: {callees:#?}"
        );
        assert!(
            callees.iter().any(|c| c.name == "index_repo"),
            "ensure_indexed should have index_repo as callee, got: {callees:#?}"
        );

        let oor_callees = db
            .get_callees("open_or_index", "src/main.rs", false)
            .unwrap();
        assert!(
            oor_callees
                .iter()
                .any(|c| c.name == "open" && c.impl_type.as_deref() == Some("Database")),
            "open_or_index should have Database::open as callee, got: {oor_callees:#?}"
        );
        assert!(
            oor_callees.iter().any(|c| c.name == "ensure_indexed"),
            "open_or_index should have ensure_indexed as callee, got: {oor_callees:#?}"
        );
    }
}
