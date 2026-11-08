//! Implements `#[derive(DiffSerializeJson)]` macro to provide a JSON-oriented
//! `serde::Serialize` impl that omits any field equal to its `Default` value.
//! Unlike `#[serde(skip_serializing_if = "…")]`, the predicate is derived
//! automatically — no hand-written helper per field is required.
//!
//! Supported `#[serde(...)]` attributes:
//! - container: `rename_all = "…"`
//! - field: `rename`, `rename(serialize = "…")`, `skip`, `skip_serializing`,
//!   `flatten`
//!
//! `serialize_with` / `with` are silently ignored, matching the previous
//! behavior where no serde attribute was interpreted at all.
//!
//! `flatten` follows serde semantics — the inner value's JSON object is
//! spliced into the parent — except that the inner value is first passed
//! through the same diff rule. An inner object that serializes to `{}` is
//! treated as “equal to default” and contributes nothing. `flatten` cannot
//! be combined with `skip_serializing_if`, mirroring serde's own conflict.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Error, Fields, Ident, LitStr, Result,
    parse_macro_input,
};

#[derive(Default)]
struct ContainerAttrs {
    rename_all: Option<RenameRule>,
}

#[derive(Default)]
struct FieldAttrs {
    /// Resolved serialization name (field-level `rename` wins over
    /// container `rename_all`).
    rename: Option<String>,
    skip: bool,
    skip_serializing: bool,
    flatten: bool,
    has_skip_serializing_if: bool,
}

struct FieldPlan {
    ident: Ident,
    attrs: FieldAttrs,
    /// Final JSON key, or `None` for `flatten` fields.
    key: Option<String>,
}

#[derive(Clone, Copy)]
enum RenameRule {
    Lowercase,
    Uppercase,
    PascalCase,
    CamelCase,
    SnakeCase,
    ScreamingSnakeCase,
    KebabCase,
    ScreamingKebabCase,
}

impl RenameRule {
    fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "lowercase" => Self::Lowercase,
            "UPPERCASE" => Self::Uppercase,
            "PascalCase" => Self::PascalCase,
            "camelCase" => Self::CamelCase,
            "snake_case" => Self::SnakeCase,
            "SCREAMING_SNAKE_CASE" => Self::ScreamingSnakeCase,
            "kebab-case" => Self::KebabCase,
            "SCREAMING-KEBAB-CASE" => Self::ScreamingKebabCase,
            _ => return None,
        })
    }

    fn apply(self, field: &str) -> String {
        match self {
            Self::Lowercase => field.to_ascii_lowercase(),
            Self::Uppercase => field.to_ascii_uppercase(),
            Self::PascalCase => split_words(field).into_iter().map(capitalize).collect(),
            Self::CamelCase => {
                let mut words = split_words(field).into_iter();
                let first = words
                    .next()
                    .map(|w| w.to_ascii_lowercase())
                    .unwrap_or_default();
                first + &words.map(capitalize).collect::<String>()
            }
            Self::SnakeCase => split_words(field)
                .into_iter()
                .map(|w| w.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("_"),
            Self::ScreamingSnakeCase => split_words(field)
                .into_iter()
                .map(|w| w.to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join("_"),
            Self::KebabCase => split_words(field)
                .into_iter()
                .map(|w| w.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("-"),
            Self::ScreamingKebabCase => split_words(field)
                .into_iter()
                .map(|w| w.to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join("-"),
        }
    }
}

fn capitalize(s: String) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => s,
    }
}

/// Split a Rust identifier into words on `_` and on lower→upper boundaries.
fn split_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut prev_lower_or_digit = false;
    for ch in s.chars() {
        if ch == '_' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            prev_lower_or_digit = false;
        } else if ch.is_ascii_uppercase() && prev_lower_or_digit {
            words.push(std::mem::take(&mut current));
            current.push(ch);
            prev_lower_or_digit = false;
        } else {
            current.push(ch);
            prev_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn parse_container_attrs(attrs: &[Attribute]) -> Result<ContainerAttrs> {
    let mut out = ContainerAttrs::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                let lit: LitStr = meta.value()?.parse()?;
                out.rename_all = Some(
                    RenameRule::from_str(&lit.value())
                        .ok_or_else(|| Error::new(lit.span(), "unknown rename_all convention"))?,
                );
                Ok(())
            } else {
                consume_meta(meta)
            }
        })?;
    }
    Ok(out)
}

fn parse_field_attrs(attrs: &[Attribute]) -> Result<FieldAttrs> {
    let mut out = FieldAttrs::default();
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                // Handles:
                //   rename = "x"
                //   rename(serialize = "x")
                //   rename(deserialize = "y")            — ignored
                //   rename(serialize = "x", deserialize = "y")
                if meta.input.peek(syn::Token![=]) {
                    let lit: LitStr = meta.value()?.parse()?;
                    out.rename = Some(lit.value());
                    Ok(())
                } else if meta.input.peek(syn::token::Paren) {
                    let content;
                    syn::parenthesized!(content in meta.input);
                    let nested = content.parse_terminated(
                        |input| {
                            let key: Ident = input.parse()?;
                            input.parse::<syn::Token![=]>()?;
                            let lit: LitStr = input.parse()?;
                            Ok((key, lit))
                        },
                        syn::Token![,],
                    )?;
                    for (key, lit) in nested {
                        if key == "serialize" {
                            out.rename = Some(lit.value());
                        }
                    }
                    Ok(())
                } else {
                    Ok(())
                }
            } else if meta.path.is_ident("skip") {
                out.skip = true;
                Ok(())
            } else if meta.path.is_ident("skip_serializing") {
                out.skip_serializing = true;
                Ok(())
            } else if meta.path.is_ident("skip_serializing_if") {
                out.has_skip_serializing_if = true;
                // The path value is parsed but unused — the diff predicate
                // strictly subsumes it. Kept only to detect the `flatten`
                // conflict below.
                if meta.input.peek(syn::Token![=]) {
                    let _: syn::LitStr = meta.value()?.parse()?;
                }
                Ok(())
            } else if meta.path.is_ident("flatten") {
                out.flatten = true;
                Ok(())
            } else {
                // `serialize_with`, `with`, `deserialize_with`, `borrow`,
                // `default`, `alias`, `bound`, and any future serde keys
                // are silently ignored.
                consume_meta(meta)
            }
        })?;
    }
    Ok(out)
}

/// Drain a `serde` meta item whose value we do not interpret.
fn consume_meta(meta: syn::meta::ParseNestedMeta<'_>) -> Result<()> {
    if meta.input.peek(syn::Token![=]) {
        let _: syn::Lit = meta.value()?.parse()?;
    } else if meta.input.peek(syn::token::Paren) {
        let _content;
        let _ = syn::parenthesized!(_content in meta.input);
    }
    Ok(())
}

pub fn derive_different_serialize_json(input: TokenStream) -> TokenStream {
    match derive_impl(parse_macro_input!(input as DeriveInput)) {
        Ok(ts) => ts.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn derive_impl(input: DeriveInput) -> Result<proc_macro2::TokenStream> {
    let name = &input.ident;

    let named = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => named,
            _ => {
                return Err(Error::new_spanned(
                    name,
                    "#[derive(DiffSerializeJson)] requires a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(Error::new_spanned(
                &input.ident,
                "#[derive(DiffSerializeJson)] can only be applied to structs",
            ));
        }
    };

    let container = parse_container_attrs(&input.attrs)?;

    let mut fields: Vec<FieldPlan> = Vec::with_capacity(named.named.len());
    for field in &named.named {
        let ident = field.ident.clone().expect("named field");
        let attrs = parse_field_attrs(&field.attrs)?;

        if attrs.flatten && attrs.has_skip_serializing_if {
            return Err(Error::new_spanned(
                &field.ident,
                "#[serde(flatten)] can not be combined with \
                 #[serde(skip_serializing_if = \"…\")]",
            ));
        }

        let key = if attrs.flatten {
            None
        } else {
            let base = attrs.rename.clone().unwrap_or_else(|| ident.to_string());
            Some(match (&attrs.rename, container.rename_all) {
                (Some(_), _) => base, // field rename overrides rename_all
                (None, Some(rule)) => rule.apply(&base),
                (None, None) => base,
            })
        };

        fields.push(FieldPlan { ident, attrs, key });
    }

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let mut stmts = Vec::with_capacity(fields.len());

    for field in &fields {
        let ident = &field.ident;

        if field.attrs.skip || field.attrs.skip_serializing {
            continue;
        }

        if field.attrs.flatten {
            stmts.push(quote! {
                {
                    let __flattened = ::serde_json::to_value(&self.#ident)
                        .map_err(::serde::ser::Error::custom)?;
                    if let ::serde_json::Value::Object(__inner) = __flattened {
                        for (__k, __v) in __inner {
                            map.serialize_entry(&__k, &__v)
                                .map_err(::serde::ser::Error::custom)?;
                        }
                    }
                }
            });
            continue;
        }

        let key_lit = field.key.as_deref().unwrap();
        stmts.push(quote! {
            if self.#ident != <_ as ::core::default::Default>::default() {
                map.serialize_entry(#key_lit, &self.#ident)
                    .map_err(::serde::ser::Error::custom)?;
            }
        });
    }

    let output = quote! {
        #[automatically_derived]
        impl #impl_generics ::serde::Serialize for #name #ty_generics #where_clause {
            fn serialize<S>(
                &self,
                serializer: S,
            ) -> ::core::result::Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                #[allow(unused_mut)]
                let mut map = serializer.serialize_map(::core::option::Option::None)?;

                #(#stmts)*

                map.end()
            }
        }
    };

    Ok(output)
}