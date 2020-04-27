use crate::{query::QueryContext, selection::Selection, TargetLang};
use failure::*;
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use std::{cell::Cell, collections::BTreeSet};

/// A GraphQL union (simplified schema representation).
///
/// For code generation purposes, unions will "flatten" fragment spreads, so
/// there is only one enum for the selection. See the tests in the
/// graphql_client crate for examples.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GqlUnion<'schema> {
    pub name: &'schema str,
    pub description: Option<&'schema str>,
    pub variants: BTreeSet<&'schema str>,
    pub is_required: Cell<bool>,
}

#[derive(Debug, Fail)]
#[fail(display = "UnionError")]
enum UnionError {
    #[fail(display = "Unknown type: {}", ty)]
    UnknownType { ty: String },
    #[fail(display = "Missing __typename in selection for {}", union_name)]
    MissingTypename { union_name: String },
}

type UnionVariantResult<'selection> =
    Result<(Vec<TokenStream>, Vec<TokenStream>, Vec<&'selection str>), failure::Error>;

/// Returns a triple.
///
/// - The first element is the union variants to be inserted directly into the
///   `enum` declaration.
/// - The second is the structs for each variant's sub-selection
/// - The last one contains which fields have been selected on the union, so we
///   can make the enum exhaustive by complementing with those missing.
pub(crate) fn union_variants<'selection>(
    target_lang: &TargetLang,
    selection: &'selection Selection<'_>,
    context: &'selection QueryContext<'selection, 'selection>,
    prefix: &str,
    selection_on: &str,
) -> UnionVariantResult<'selection> {
    let selection = selection.selected_variants_on_union(context, selection_on)?;
    let mut used_variants: Vec<&str> = selection.keys().cloned().collect();
    let mut children_definitions = Vec::with_capacity(selection.len());
    let mut variants = Vec::with_capacity(selection.len());

    for (on, fields) in selection.iter() {
        let variant_name = Ident::new(&on, Span::call_site());
        used_variants.push(on);

        let new_prefix = format!("{}On{}", prefix, on);

        let variant_type = Ident::new(&new_prefix, Span::call_site());

        let field_object_type = context
            .schema
            .objects
            .get(on)
            .map(|_f| context.maybe_expand_field(target_lang, &on, fields, &new_prefix));
        let field_interface = context
            .schema
            .interfaces
            .get(on)
            .map(|_f| context.maybe_expand_field(target_lang, &on, fields, &new_prefix));
        let field_union_type = context
            .schema
            .unions
            .get(on)
            .map(|_f| context.maybe_expand_field(target_lang, &on, fields, &new_prefix));

        match field_object_type.or(field_interface).or(field_union_type) {
            Some(Ok(Some(tokens))) => children_definitions.push(tokens),
            Some(Err(err)) => Err(err)?,
            Some(Ok(None)) => (),
            None => Err(UnionError::UnknownType { ty: on.to_string() })?,
        };

        variants.push(match target_lang {
            TargetLang::Rust => quote! { #variant_name(#variant_type) },
            TargetLang::Go => unimplemented!(),
        })
    }

    Ok((variants, children_definitions, used_variants))
}

impl<'schema> GqlUnion<'schema> {
    /// Returns the code to deserialize this union in the response given the
    /// query selection.
    pub(crate) fn response_for_selection(
        &self,
        target_lang: &TargetLang,
        query_context: &QueryContext<'_, '_>,
        selection: &Selection<'_>,
        prefix: &str,
    ) -> Result<TokenStream, failure::Error> {
        let typename_field = selection.extract_typename(query_context);

        if typename_field.is_none() {
            Err(UnionError::MissingTypename {
                union_name: prefix.into(),
            })?;
        }

        let struct_name = Ident::new(prefix, Span::call_site());
        let derives = query_context.response_derives();

        let (mut variants, children_definitions, used_variants) =
            union_variants(target_lang, selection, query_context, prefix, &self.name)?;

        variants.extend(
            self.variants
                .iter()
                .filter(|v| used_variants.iter().find(|a| a == v).is_none())
                .map(|v| {
                    let v = Ident::new(v, Span::call_site());
                    quote!(#v).into()
                }),
        );

        match target_lang {
            TargetLang::Rust => Ok(quote! {
                #(#children_definitions)*

                #derives
                #[serde(tag = "__typename")]
                pub enum #struct_name {
                    #(#variants),*
                }
            }),
            TargetLang::Go => unimplemented!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        constants::*,
        deprecation::DeprecationStatus,
        field_type::FieldType,
        objects::{GqlObject, GqlObjectField},
        selection::*,
    };

    #[test]
    fn union_response_for_selection_complains_if_typename_is_missing() {
        let fields = vec![
            SelectionItem::InlineFragment(SelectionInlineFragment {
                on: "User",
                fields: Selection::from_vec(vec![SelectionItem::Field(SelectionField {
                    alias: None,
                    name: "firstName",
                    fields: Selection::new_empty(),
                })]),
            }),
            SelectionItem::InlineFragment(SelectionInlineFragment {
                on: "Organization",
                fields: Selection::from_vec(vec![SelectionItem::Field(SelectionField {
                    alias: None,
                    name: "title",
                    fields: Selection::new_empty(),
                })]),
            }),
        ];
        let selection = Selection::from_vec(fields);
        let prefix = "Meow";
        let union = GqlUnion {
            name: "MyUnion",
            description: None,
            variants: BTreeSet::new(),
            is_required: false.into(),
        };

        let mut schema = crate::schema::Schema::new();

        schema.objects.insert(
            "User",
            GqlObject {
                description: None,
                name: "User",
                fields: vec![
                    GqlObjectField {
                        description: None,
                        name: "firstName",
                        type_: FieldType::new("String").nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                    GqlObjectField {
                        description: None,
                        name: "lastName",
                        type_: FieldType::new("String").nonnull(),

                        deprecation: DeprecationStatus::Current,
                    },
                    GqlObjectField {
                        description: None,
                        name: "createdAt",
                        type_: FieldType::new("Date").nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                ],
                is_required: false.into(),
            },
        );

        schema.objects.insert(
            "Organization",
            GqlObject {
                description: None,
                name: "Organization",
                fields: vec![
                    GqlObjectField {
                        description: None,
                        name: "title",
                        type_: FieldType::new("String").nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                    GqlObjectField {
                        description: None,
                        name: "created_at",
                        type_: FieldType::new("Date").nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                ],
                is_required: false.into(),
            },
        );
        let context = QueryContext::new_empty(&schema);

        let result = union.response_for_selection(&context, &selection, &prefix);

        assert!(result.is_err());

        assert_eq!(
            format!("{}", result.unwrap_err()),
            "Missing __typename in selection for Meow"
        );
    }

    #[test]
    fn union_response_for_selection_works() {
        let fields = vec![
            SelectionItem::Field(SelectionField {
                alias: None,
                name: "__typename",
                fields: Selection::new_empty(),
            }),
            SelectionItem::InlineFragment(SelectionInlineFragment {
                on: "User",
                fields: Selection::from_vec(vec![SelectionItem::Field(SelectionField {
                    alias: None,
                    name: "firstName",
                    fields: Selection::new_empty(),
                })]),
            }),
            SelectionItem::InlineFragment(SelectionInlineFragment {
                on: "Organization",
                fields: Selection::from_vec(vec![SelectionItem::Field(SelectionField {
                    alias: None,
                    name: "title",
                    fields: Selection::new_empty(),
                })]),
            }),
        ];
        let schema = crate::schema::Schema::new();
        let context = QueryContext::new_empty(&schema);
        let selection: Selection<'_> = fields.into_iter().collect();
        let prefix = "Meow";
        let union = GqlUnion {
            name: "MyUnion",
            description: None,
            variants: BTreeSet::new(),
            is_required: false.into(),
        };

        let result = union.response_for_selection(&context, &selection, &prefix);

        assert!(result.is_err());

        let mut schema = crate::schema::Schema::new();
        schema.objects.insert(
            "User",
            GqlObject {
                description: None,
                name: "User",
                fields: vec![
                    GqlObjectField {
                        description: None,
                        name: "__typename",
                        type_: FieldType::new(string_type()).nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                    GqlObjectField {
                        description: None,
                        name: "firstName",
                        type_: FieldType::new(string_type()).nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                    GqlObjectField {
                        description: None,
                        name: "lastName",
                        type_: FieldType::new(string_type()).nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                    GqlObjectField {
                        description: None,
                        name: "createdAt",
                        type_: FieldType::new("Date").nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                ],
                is_required: false.into(),
            },
        );

        schema.objects.insert(
            "Organization",
            GqlObject {
                description: None,
                name: "Organization",
                fields: vec![
                    GqlObjectField {
                        description: None,
                        name: "__typename",
                        type_: FieldType::new(string_type()).nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                    GqlObjectField {
                        description: None,
                        name: "title",
                        type_: FieldType::new("String").nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                    GqlObjectField {
                        description: None,
                        name: "createdAt",
                        type_: FieldType::new("Date").nonnull(),
                        deprecation: DeprecationStatus::Current,
                    },
                ],
                is_required: false.into(),
            },
        );

        let context = QueryContext::new_empty(&schema);

        let result = union.response_for_selection(&context, &selection, &prefix);

        println!("{:?}", result);

        assert!(result.is_ok());

        assert_eq!(
            result.unwrap().to_string(),
            vec![
                "# [ derive ( Deserialize ) ] ",
                "pub struct MeowOnOrganization { pub title : String , } ",
                "# [ derive ( Deserialize ) ] ",
                "pub struct MeowOnUser { # [ serde ( rename = \"firstName\" ) ] pub first_name : String , } ",
                "# [ derive ( Deserialize ) ] ",
                "# [ serde ( tag = \"__typename\" ) ] ",
                "pub enum Meow { Organization ( MeowOnOrganization ) , User ( MeowOnUser ) }",
            ].into_iter()
                .collect::<String>(),
        );
    }
}
