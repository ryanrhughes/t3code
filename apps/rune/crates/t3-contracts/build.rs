//! Generates the contract types from `schema/contracts.json`, which
//! `scripts/export-contract-schemas.ts` writes from `packages/contracts`.
use std::path::PathBuf;

use anyhow::{Context, Result};
use quote::quote;
use serde_json::Value;
use syn::visit_mut::VisitMut;

fn main() -> Result<()> {
    let schema_path = PathBuf::from("schema/contracts.json");
    println!("cargo:rerun-if-changed={}", schema_path.display());
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&schema_path)?)?;
    let rpcs = doc.get("x-rpcs").cloned().context("no x-rpcs")?;
    let mut defs = doc
        .as_object_mut()
        .and_then(|o| o.remove("$defs"))
        .context("no $defs")?;
    let tags = def_tags(&defs);
    let mut untyped = 0;
    prepare(&mut defs, &tags, &mut untyped);
    if untyped > 0 {
        println!("cargo:warning={untyped} contract unions repeat a tag and stay untyped JSON");
    }

    // typify reads schemas through schemars 0.8, which knows draft-07 `definitions`.
    let root: schemars::schema::RootSchema =
        serde_json::from_value(serde_json::json!({ "definitions": defs }))?;
    let mut settings = typify::TypeSpaceSettings::default();
    settings.with_struct_builder(false);
    let mut space = typify::TypeSpace::new(&settings);
    space
        .add_root_schema(root)
        .context("typify rejected the schema")?;
    let mut file =
        syn::parse2::<syn::File>(space.to_stream()).context("generated code didn't parse")?;
    // Scalar unions get exhaustive `Display`/`FromStr` impls from typify; they keep their variants.
    let scalar: std::collections::HashSet<String> = file
        .items
        .iter()
        .filter_map(|i| match i {
            syn::Item::Impl(imp)
                if imp.trait_.as_ref().is_some_and(|(_, p, _)| {
                    p.segments
                        .last()
                        .is_some_and(|s| s.ident == "Display" || s.ident == "FromStr")
                }) =>
            {
                match &*imp.self_ty {
                    syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect();
    Adjust { scalar }.visit_file_mut(&mut file);

    let mut text = prettyplease::unparse(&file);
    text.push_str(&rpc_table(&rpcs));
    std::fs::write(
        PathBuf::from(std::env::var("OUT_DIR")?).join("generated.rs"),
        text,
    )?;
    Ok(())
}

/// Rewrites `$defs` refs for schemars, reads unions as `oneOf`, and removes keywords typify can't use that don't change
/// the wire shape: string patterns (validation only), Effect annotations, and `not` (Effect writes
/// `not: {type: null}` for "anything but null" and `not: {}` for `Never`). Enum values Rust can't
/// name apart (`"#"`, `"!"`) stay plain strings.
fn prepare(
    node: &mut Value,
    tags: &std::collections::HashMap<String, String>,
    untyped: &mut usize,
) {
    match node {
        Value::Object(map) => {
            map.retain(|k, _| {
                !matches!(
                    k.as_str(),
                    "pattern" | "dialect" | "examples" | "default" | "not"
                )
            });
            // Effect writes unions as `anyOf`, but one member matches each value; typify reads
            // `anyOf` as "any combination" and builds flattened structs that can't hold a string.
            if let Some(Value::Array(mut members)) = map.remove("anyOf") {
                let mut seen = std::collections::HashSet::new();
                members.retain(|m| seen.insert(m.to_string()));
                // A union of unions can repeat a tag with different shapes; typify can't name
                // those variants apart, so the value stays `serde_json::Value`.
                let names: Vec<String> =
                    members.iter().filter_map(|m| member_tag(m, tags)).collect();
                if names.len() != names.iter().collect::<std::collections::HashSet<_>>().len() {
                    *untyped += 1;
                    map.clear();
                    return;
                }
                map.insert("oneOf".into(), Value::Array(members));
            }
            if let Some(Value::String(r)) = map.get_mut("$ref") {
                *r = r.replace("#/$defs/", "#/definitions/");
            }
            if let Some(Value::Array(values)) = map.get("enum") {
                let names: Vec<String> = values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|v| {
                        v.chars()
                            .filter(char::is_ascii_alphanumeric)
                            .collect::<String>()
                            .to_ascii_lowercase()
                    })
                    .collect();
                let unique: std::collections::HashSet<_> = names.iter().collect();
                if names.iter().any(String::is_empty) || unique.len() != names.len() {
                    map.remove("enum");
                }
            }
            map.values_mut().for_each(|v| prepare(v, tags, untyped));
        }
        Value::Array(items) => items.iter_mut().for_each(|v| prepare(v, tags, untyped)),
        _ => {}
    }
}

/// Each definition's union tag (`type`, `kind` or `_tag` with one value), by definition name.
fn def_tags(defs: &Value) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for (name, def) in defs.as_object().into_iter().flatten() {
        if let Some(tag) = inline_tag(def) {
            out.insert(name.clone(), tag);
        }
    }
    out
}

fn inline_tag(node: &Value) -> Option<String> {
    let props = node.get("properties")?;
    ["type", "kind", "_tag"].iter().find_map(|t| {
        let values = props.get(*t)?.get("enum")?.as_array()?;
        (values.len() == 1)
            .then(|| values[0].as_str().map(str::to_owned))
            .flatten()
    })
}

fn member_tag(member: &Value, tags: &std::collections::HashMap<String, String>) -> Option<String> {
    match member.get("$ref").and_then(Value::as_str) {
        Some(r) => tags.get(r.rsplit('/').next()?).cloned(),
        None => inline_tag(member),
    }
}

/// Drops typify's schema-dump doc comments, and gives every internally tagged enum an `Unknown`
/// fallback: a variant a newer server adds decodes instead of failing the whole frame, as the
/// TypeScript client's `ForwardCompatibleUnion` does.
struct Adjust {
    scalar: std::collections::HashSet<String>,
}

impl VisitMut for Adjust {
    fn visit_attributes_mut(&mut self, attrs: &mut Vec<syn::Attribute>) {
        attrs.retain(|a| !a.path().is_ident("doc"));
    }

    fn visit_item_enum_mut(&mut self, item: &mut syn::ItemEnum) {
        syn::visit_mut::visit_item_enum_mut(self, item);
        let tagged = item.attrs.iter().any(|a| {
            a.path().is_ident("serde")
                && a.meta
                    .require_list()
                    .is_ok_and(|l| l.tokens.to_string().contains("tag ="))
        });
        let serde_list = |needle: &str| {
            item.attrs.iter().any(|a| {
                a.path().is_ident("serde")
                    && a.meta
                        .require_list()
                        .is_ok_and(|l| l.tokens.to_string().contains(needle))
            })
        };
        if item.variants.iter().any(|v| v.ident == "Unknown") {
            return;
        }
        if tagged {
            item.variants
                .push(syn::parse_quote! { #[serde(other)] Unknown });
            return;
        }
        // An untagged union takes its first matching member; real traffic can carry members the
        // contract no longer lists (stored legacy values), so the last resort is raw JSON. Enums
        // that derive `Eq` or `Hash` can't hold a `Value` and stay strict.
        let derives = item
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("derive"))
            .map(|a| {
                a.meta
                    .require_list()
                    .map(|l| l.tokens.to_string())
                    .unwrap_or_default()
            })
            .collect::<String>();
        let strict = derives.contains("Eq") || derives.contains("Hash") || derives.contains("Ord");
        if serde_list("untagged") && !strict && !self.scalar.contains(&item.ident.to_string()) {
            item.variants
                .push(syn::parse_quote! { Unknown(::serde_json::Value) });
        }
    }
}

/// `RPCS`: every method's wire tag, whether it streams, and its generated type-name stem.
fn rpc_table(rpcs: &Value) -> String {
    let rows = rpcs.as_array().into_iter().flatten().map(|r| {
        let (tag, stream, name) = (
            r["tag"].as_str().unwrap_or_default(),
            r["stream"].as_bool().unwrap_or(false),
            r["name"].as_str().unwrap_or_default(),
        );
        quote! { (#tag, #stream, #name) }
    });
    let tokens = quote! {
        /// Every WebSocket RPC: wire tag, whether it streams, and the generated type-name stem
        /// (`<Stem>Payload`, `<Stem>Success` or `<Stem>Item`, `<Stem>Error`).
        pub const RPCS: &[(&str, bool, &str)] = &[#(#rows),*];
    };
    format!("\n{tokens}\n")
}
