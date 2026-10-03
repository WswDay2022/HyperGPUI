use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Ident};

/// Derives `hgpui::ElementDelegate` by delegating to an inner field.
///
/// Defaults to the `element` field. Override with `#[delegate(field = name)]`.
pub fn derive_element_delegate(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let type_name = &ast.ident;

    // Resolve target field: explicit `field = ...` or default `element`.
    let field_name = parse_field_attr(&ast)
        .ok()
        .flatten()
        .unwrap_or_else(|| Ident::new("element", type_name.span()));

    // Look up the field to obtain its type.
    let field_type = match &ast.data {
        Data::Struct(s) => s
            .fields
            .iter()
            .find(|f| f.ident.as_ref() == Some(&field_name))
            .map(|f| f.ty.clone()),
        _ => None,
    };

    let Some(field_type) = field_type else {
        return syn::Error::new_spanned(
            type_name,
            format!("no field named `{field_name}`"),
        )
            .to_compile_error()
            .into();
    };

    let (impl_generics, type_generics, where_clause) = ast.generics.split_for_impl();

    let r#gen = quote! {
        impl #impl_generics hgpui::ElementDelegate
            for #type_name #type_generics #where_clause
        {
            type Target = #field_type;
            fn delegate(&mut self) -> &mut #field_type {
                &mut self.#field_name
            }
        }
    };

    r#gen.into()
}

/// Parses `#[delegate(field = ident)]` from the struct attributes.
fn parse_field_attr(ast: &DeriveInput) -> syn::Result<Option<Ident>> {
    for attr in &ast.attrs {
        if attr.path().is_ident("delegate") {
            let mut result = None;
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("field") {
                    let value = meta.value()?;               // eat `=`
                    result = Some(value.parse::<Ident>()?);  // parse ident
                }
                Ok(())
            })?;
            return Ok(result);
        }
    }
    Ok(None)
}