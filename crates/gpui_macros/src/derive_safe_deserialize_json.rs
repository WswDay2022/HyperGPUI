//! Fail-safe JSON deserialization via `#[derive(SafeDeserializeJson)]`.
//!
//! This macro generates a `serde::Deserialize` implementation that
//! **never returns `Err`**. Any failure — malformed JSON, a non-object
//! root, or a single field of the wrong type — is swallowed, and the
//! affected field(s) silently fall back to `Default::default()`.
//!
//! # Quick example
//!
//! ```ignore
//! use my_crate::SafeDeserializeJson;
//!
//! #[derive(SafeDeserializeJson, Default, Debug, PartialEq)]
//! struct User {
//!     id: u32,
//!     name: String,
//!     tags: Vec<String>,
//! }
//!
//! // Well-formed input behaves as expected.
//! let u: User = serde_json::from_str(r#"{"id": 42, "name": "alice"}"#).unwrap();
//! assert_eq!(u, User { id: 42, name: "alice".into(), tags: vec![] });
//!
//! // A single bad field only resets that field.
//! let u: User = serde_json::from_str(r#"{"id": 42, "name": 99}"#).unwrap();
//! assert_eq!(u, User { id: 42, name: String::new(), tags: vec![] });
//!
//! // Anything else yields the all-default value; no `Err` is ever returned.
//! let u: User = serde_json::from_str(r#"not json"#).unwrap();
//! assert_eq!(u, User::default());
//! ```
//!
//! # Behaviour contract
//!
//! The generated `deserialize` is exactly equivalent to the following
//! pseudo-code (where `field₁…fieldₙ` are the struct's named fields, in
//! declaration order):
//!
//! ```text
//! fn deserialize(deserializer) -> Result<Self, D::Error> {
//!     let mut result = Self::default();
//!     let value = match serde_json::Value::deserialize(deserializer) {
//!         Ok(v)  => v,
//!         Err(_) => return Ok(result),   // (1)
//!     };
//!     let mut object = match value {
//!         Value::Object(m) => m,
//!         _                => return Ok(result),   // (2)
//!     };
//!     for f in [field₁, …, fieldₙ] {
//!         if let Some(v) = object.remove(stringify!(f))
//!             && let Ok(parsed) = serde_json::from_value(v) {
//!             result.f = parsed;
//!         }
//!     }
//!     Ok(result)   // (3)
//! }
//! ```
//!
//! The numbered points deserve elaboration:
//!
//! 1. **Unparseable input.** If the deserializer cannot produce a
//!    `serde_json::Value` — which for `serde_json::from_str` /
//!    `from_slice` / `from_value` means malformed JSON — the whole
//!    result is `Self::default()`.
//!
//! 2. **Non-object root.** JSON `null`, `true`, `42`, `"x"`, or `[…]`
//!    at the top level also yields `Self::default()`. Only a top-level
//!    `{…}` contributes anything.
//!
//! 3. **Per-field isolation.** Fields are handled independently.
//!    - Key **absent** from the input object → field keeps its default.
//!    - Key **present** but the value fails to deserialize into the
//!      field's type → field keeps its default. Other fields are
//!      unaffected.
//!    - Key **present and valid** → field is assigned.
//!
//!    Note that this fallback is **shallow**: if a field's type is
//!    itself a struct with a `#[derive(Deserialize)]` impl, a type
//!    error *inside* that struct causes `from_value` to return `Err`
//!    and the **whole outer field** falls back to its default. The
//!    fallback does not recurse into nested types. If you want nested
//!    fallback, derive `SafeDeserializeJson` on the inner type as well.
//!
//! 4. **Unknown keys.** Any key in the input object that does not match
//!    a field's name is silently discarded. `#[serde(deny_unknown_fields)]`
//!    is not interpreted and has no effect.
//!
//! 5. **Never `Err`.** The only `Err` the function can produce would
//!    have to come from `Self::default()`, which returns `Self` directly.
//!    In practice this means the `Result` return type exists solely to
//!    satisfy the `serde::Deserialize` trait signature.
//!
//! # Key resolution: `stringify!(field)`
//!
//! The JSON key looked up for each field is the **Rust identifier**
//! passed through `stringify!`, verbatim. There is no renaming,
//! normalization, or casing rule. Consequently:
//!
//! - A field named `id` is read from key `"id"`.
//! - A field named `user_id` is read from key `"user_id"`.
//! - A field named `r#type` is read from key **`"r#type"`**, *not*
//!   `"type"`. The `r#` prefix is part of the `stringify!` output. This
//!   is a common foot-gun when a struct uses a raw identifier to dodge
//!   a Rust keyword but the JSON producer uses the natural name. If you
//!   need a different key, either rename the Rust field or wrap the
//!   target type in a newtype with a hand-written `Deserialize`.
//!
//! # `#[serde(...)]` attributes: not interpreted at all
//!
//! This macro does **not** read any `#[serde]` attribute on the
//! container or on individual fields. Every attribute is ignored
//! entirely. The consequences, spelled out per attribute, are:
//!
//! | Attribute                      | What happens here                                    |
//! |--------------------------------|------------------------------------------------------|
//! | `rename`, `rename_all`         | Ignored. The key is always `stringify!(field)`.     |
//! | `alias`                        | Ignored. Only the field's own name is looked up.    |
//! | `skip`, `skip_deserializing`   | Ignored. The field **is** looked up in the input.   |
//! | `default = "fn"`               | Ignored. Fallback is always `Self::default()`;      |
//! |                                | the field's own `Default::default()` is used.       |
//! | `flatten`                      | Ignored. The field has no key of its own and will   |
//! |                                | therefore always keep its default value.            |
//! | `deserialize_with`, `with`     | Ignored. `from_value` calls the type's own          |
//! |                                | `Deserialize` impl; the custom function is skipped. |
//! | `borrow`, `bound`              | Ignored.                                             |
//! | `deny_unknown_fields`          | Ignored. Unknown keys are always discarded.         |
//!
//! The `skip` case is the subtlest: under `#[derive(Deserialize)]` a
//! skipped field is never touched. Under this macro it *is* looked up,
//! and if a same-named key happens to be present it will be assigned.
//! This cannot panic or fail — the worst case is an unexpected value —
//! but it is a semantic difference worth knowing about if you are
//! migrating a struct that previously carried `#[serde(skip)]`.
//!
//! # Comparison with `#[derive(Deserialize)]`
//!
//! | Aspect                        | `#[derive(Deserialize)]`           | `SafeDeserializeJson`              |
//! |-------------------------------|------------------------------------|------------------------------------|
//! | On malformed JSON             | `Err`                              | `Ok(Self::default())`              |
//! | On non-object root            | `Err`                              | `Ok(Self::default())`              |
//! | On field type mismatch        | `Err` (whole struct)               | `Ok(…partial defaults…)`           |
//! | On unknown key                | Ignored by default                 | Ignored                            |
//! | `#[serde]` attributes         | Fully honoured                     | Fully ignored                      |
//! | Key resolution                | serde rules + `rename`, etc.       | `stringify!(field)` only           |
//! | Accepts non-JSON deserializer | Yes                                | Only if it can produce a `Value`   |
//! | Zero-copy field borrow        | Possible (`#[serde(borrow)]`)      | No: everything goes through `Value`|
//!
//! # Relationship to `impl_safe_deserialize!`
//!
//! This macro is the procedural-macro successor to the older
//! `impl_safe_deserialize!` `macro_rules!` macro. Its runtime behaviour
//! is intended to be identical: non-object roots, malformed input, and
//! per-field type errors all fall back to `Default`, and the generated
//! `deserialize` never returns `Err`. It also deliberately drops the
//! `#[serde(skip)]` recognition that an intermediate revision added —
//! the whole point of this version is that the surface is small and the
//! semantics are easy to reason about.
//!
//! # Requirements on the type
//!
//! - The target must be a **struct with named fields**. Unit structs,
//!   tuple structs, enums, and unions are rejected with a compile error.
//! - The target must implement **`Default`** (usually via
//!   `#[derive(Default)]`). It is used to seed `result` and to short-
//!   circuit the two early returns.
//! - Every **field type** must implement `serde::de::DeserializeOwned`,
//!   because `serde_json::from_value` requires it. This is not stated
//!   in the generated `where` clause — the bound is inferred by the
//!   compiler at each `from_value` call site. For concrete field types
//!   this is invisible; for generic structs you may need to add the
//!   bound yourself (see next section).
//!
//! # Generics and lifetimes
//!
//! The macro injects a fresh `'de` lifetime parameter at the front of
//! the impl's generics (lifetimes must precede type/const parameters in
//! Rust syntax). Therefore:
//!
//! - **A user-defined `'de` lifetime on the struct is rejected** with a
//!   compile error. Rename it (e.g. `'a`) and try again.
//! - Any other lifetimes, type parameters, and const parameters are
//!   passed through unchanged, and the struct's own `where` clause is
//!   preserved on the generated impl.
//!
//! For a generic struct such as `struct Wrap<T> { inner: T }`, the
//! generated `from_value::<T>` requires `T: DeserializeOwned`, but the
//! macro does not add that bound. Write it on the struct:
//!
//! ```ignore
//! #[derive(SafeDeserializeJson, Default)]
//! struct Wrap<T>
//! where
//!     T: serde::de::DeserializeOwned,
//! {
//!     inner: T,
//! }
//! ```
//!
//! # Performance characteristics
//!
//! - **One full parse into `serde_json::Value`.** The input is first
//!   deserialized into an owned `Value` tree, then each field is
//!   extracted from that tree with `from_value`. For deeply nested or
//!   very large inputs this is more expensive than a streaming
//!   `Deserializer` walk; the trade-off is that every failure mode is
//!   trivially swallowed.
//! - **No clone on the field path.** `Map::remove(&str)` moves the
//!   `Value` out of the map, and `from_value` consumes it. The
//!   per-field extraction is pure move semantics.
//! - **Two zero-cost early returns.** On malformed input or a non-
//!   object root, `result` has not yet been touched, so it is returned
//!   directly instead of being thrown away and re-constructed. This
//!   saves one potentially non-trivial `Default` construction on the
//!   failure path.
//! - **`#[allow(unused_mut, unused_variables)]` on `deserialize`.**
//!   Covers the zero-field struct case where neither `result` nor
//!   `object` ends up mutated, so the macro compiles warning-free even
//!   for `struct Empty {}`.
//!
//! # Pitfalls checklist
//!
//! - Raw identifiers (`r#type`, `r#match`, …) resolve to keys with the
//!   `r#` prefix intact.
//! - `#[serde(skip)]` no longer prevents a lookup — see the table above.
//! - Nested structs do **not** get recursive fallback; derive this
//!   macro on the inner type if you need that.
//! - Numeric precision is whatever `serde_json::Value`'s number type
//!   preserves. Extremely large `u64` values that exceed `i64::MAX` are
//   representable, but downstream conversion through `f64`-shaped
//!   `Value`s can lose precision; if you depend on exact integers,
//!   verify against your actual JSON producer.
//! - The impl is only usable with deserializers that can produce a
//!   `serde_json::Value`. For `serde_json::from_str` / `from_slice` /
//!   `from_value` this is trivially true; for other formats it usually
//!   is not, and the macro will silently produce `Self::default()`.
//!
//! # When to use this
//!
//! Good fit:
//!
//! - Telemetry / metrics structs where a partially-broken payload
//!   should be recorded with whatever fields did parse.
//! - Configuration structs where "fall back to defaults" is a
//!   documented product behaviour.
//! - Boundary types in a service that must not fail on a third-party
//!   producer's schema drift.
//!
//! Poor fit:
//!
//! - Anywhere a field error must be *reported* rather than swallowed.
//! - Structs that rely on `rename`, `flatten`, `deserialize_with`, or
//!   `deny_unknown_fields` for their correctness.
//! - Hot paths where the extra `Value` allocation matters.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, GenericParam, Ident, parse_macro_input, parse_quote};

/// Derive a fail-safe `serde::Deserialize` implementation that never
/// returns `Err` and falls back to `Default` on any error.
///
/// See the [crate-level documentation](self) for the full behaviour
/// contract, the list of ignored `#[serde]` attributes, and the
/// migration notes relative to `impl_safe_deserialize!`.
///
/// # Generated code
///
/// The macro emits an `impl<'de> serde::Deserialize<'de> for YourType`
/// whose body:
///
/// 1. seeds a `result` with `Self::default()`;
/// 2. deserializes the input into a `serde_json::Value`, returning
///    `result` unchanged if that fails;
/// 3. pattern-matches the value into an object, returning `result`
///    unchanged if it is anything else;
/// 4. for each named field, `remove`s the key `stringify!(field)` from
///    the object and assigns the field if `from_value` succeeds;
/// 5. returns `Ok(result)`.
///
/// # Requirements
///
/// - Named-field struct only.
/// - `Self: Default`.
/// - Every field type: `serde::de::DeserializeOwned` (inferred, not
///   declared).
/// - No `'de` lifetime on the struct itself.
///
/// # Examples
///
/// ```ignore
/// use my_crate::SafeDeserializeJson;
///
/// #[derive(SafeDeserializeJson, Default, Debug, PartialEq)]
/// struct Config {
///     retries: u32,
///     endpoint: String,
/// }
///
/// // A totally malformed payload does not error.
/// let c: Config = serde_json::from_str("oops").unwrap();
/// assert_eq!(c, Config::default());
///
/// // A non-object root does not error.
/// let c: Config = serde_json::from_str("[1, 2, 3]").unwrap();
/// assert_eq!(c, Config::default());
///
/// // A single bad field only resets that field.
/// let c: Config = serde_json::from_str(r#"{"retries": 3, "endpoint": null}"#).unwrap();
/// assert_eq!(c, Config { retries: 3, endpoint: String::new() });
/// ```
pub fn derive_safe_deserialize_json(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Collect every named field. Unit / tuple structs, enums, and unions
    // are rejected. No serde attribute inspection happens here — the
    // simplest possible shape is also the most predictable one.
    let field_names: Vec<&Ident> = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => named.named.iter().filter_map(|f| f.ident.as_ref()).collect(),
            _ => {
                return syn::Error::new_spanned(
                    name,
                    "#[derive(SafeDeserializeJson)] requires a struct with named fields",
                )
                    .to_compile_error()
                    .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(
                name,
                "#[derive(SafeDeserializeJson)] can only be applied to structs",
            )
                .to_compile_error()
                .into();
        }
    };

    // Reject pre-existing `'de` lifetimes — the generated impl injects one.
    let mut impl_generics = input.generics.clone();
    for param in &impl_generics.params {
        if let GenericParam::Lifetime(lt) = param
            && lt.lifetime.ident == "de"
        {
            return syn::Error::new_spanned(
                &lt.lifetime,
                "#[derive(SafeDeserializeJson)] conflicts with a user-defined `'de` lifetime",
            )
                .to_compile_error()
                .into();
        }
    }
    // Lifetime parameters must precede type/const parameters; `push` would
    // append `'de` after user type parameters and produce `impl<T, 'de>`.
    impl_generics.params.insert(0, parse_quote!('de));

    let (impl_generics_tokens, _, where_clause) = impl_generics.split_for_impl();
    let (_, ty_generics, _) = input.generics.split_for_impl();

    // One `if let ... && let ...` block per field. `Map::remove(&str)` moves
    // the `Value` out (no clone); `from_value` consumes it, so the whole
    // per-field path is pure move semantics.
    let field_assignments = field_names.iter().map(|field| {
        quote! {
            if let Some(field_value) = object.remove(stringify!(#field))
                && let Ok(parsed) = ::serde_json::from_value(field_value)
            {
                result.#field = parsed;
            }
        }
    });

    let output = quote! {
        #[automatically_derived]
        impl #impl_generics_tokens ::serde::Deserialize<'de> for #name #ty_generics #where_clause {
            // `unused_mut` covers the zero-field case where neither `result`
            // nor `object` is ever mutated; `unused_variables` covers `object`
            // being unused when there are no fields to extract. Hoisting the
            // allows to the function keeps the body attribute-free.
            #[allow(unused_mut, unused_variables)]
            fn deserialize<D>(deserializer: D) -> ::core::result::Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                let mut result = <Self as ::core::default::Default>::default();

                // Parse the whole input into a `Value`. Failures here (non-JSON
                // deserializer, malformed JSON) short-circuit to the default.
                let value = match <::serde_json::Value as ::serde::Deserialize>::deserialize(deserializer) {
                    Ok(value) => value,
                    Err(_) => return Ok(result),
                };

                // Non-object roots also yield the default. `result` has not
                // been touched yet, so returning it directly is equivalent to
                // constructing a fresh `Default::default()` — and skips one
                // potentially non-trivial `Default` construction.
                let mut object = match value {
                    ::serde_json::Value::Object(map) => map,
                    _ => return Ok(result),
                };

                #(#field_assignments)*

                Ok(result)
            }
        }
    };

    TokenStream::from(output)
}