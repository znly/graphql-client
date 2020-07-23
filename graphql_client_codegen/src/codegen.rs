use crate::{
    fragments::GqlFragment, operations::Operation, query::QueryContext, schema,
    selection::Selection, TargetLang,
};
use failure::*;
use graphql_parser::query;
use proc_macro2::TokenStream;
use quote::*;

// -----------------------------------------------------------------------------

/// Selects the first operation matching `struct_name`. Returns `None` when the
/// query document defines no operation, or when the selected operation does not
/// match any defined operation.
pub(crate) fn select_operation<'query>(
    query: &'query query::Document,
    struct_name: &str,
) -> Option<Operation<'query>> {
    use heck::CamelCase;

    let operations = all_operations(query);

    operations
        .iter()
        .find(|op| op.name == struct_name)
        .or_else(|| {
            if cfg!(feature = "normalize_query_types") {
                operations
                    .iter()
                    .find(|op| op.name.to_camel_case() == struct_name)
            } else {
                None
            }
        })
        .map(ToOwned::to_owned)
}

pub(crate) fn all_operations(query: &query::Document) -> Vec<Operation<'_>> {
    let mut operations: Vec<Operation<'_>> = Vec::new();

    for definition in &query.definitions {
        if let query::Definition::Operation(op) = definition {
            operations.push(op.into());
        }
    }
    operations
}

// -----------------------------------------------------------------------------

/// The main code generation function.
pub(crate) fn response_for_query(
    schema: &schema::Schema<'_>,
    query: &query::Document,
    operation: &Operation<'_>,
    options: &crate::GraphQLClientCodegenOptions,
) -> Result<TokenStream, failure::Error> {
    let mut context = QueryContext::new(
        schema,
        options.deprecation_strategy(),
        operation.operation_type,
    );

    if let Some(derives) = options.additional_derives() {
        context.ingest_additional_derives(&derives)?;
    }

    let mut definitions = Vec::new();

    for definition in &query.definitions {
        match definition {
            query::Definition::Operation(_op) => (),
            query::Definition::Fragment(fragment) => {
                let &query::TypeCondition::On(ref on) = &fragment.type_condition;
                let on = schema.fragment_target(on).ok_or_else(|| {
                    format_err!(
                        "Fragment {} is defined on unknown type: {}",
                        &fragment.name,
                        on,
                    )
                })?;
                context.fragments.insert(
                    &fragment.name,
                    GqlFragment {
                        name: &fragment.name,
                        selection: Selection::from(&fragment.selection_set),
                        on,
                        is_required: false.into(),
                    },
                );
            }
        }
    }

    let response_data_fields = {
        let root_name = operation.root_name(&context.schema);
        let opt_definition = context.schema.objects.get(&root_name);
        let definition = if let Some(definition) = opt_definition {
            definition
        } else {
            panic!(
                "operation type '{:?}' not in schema",
                operation.operation_type
            );
        };
        let prefix = &operation.name;
        let selection = &operation.selection;

        if operation.is_subscription() && selection.len() > 1 {
            Err(format_err!(
                "{}",
                crate::constants::MULTIPLE_SUBSCRIPTION_FIELDS_ERROR
            ))?
        }

        definitions.extend(definition.field_impls_for_selection(
            &options.target_lang,
            &context,
            &selection,
            &prefix,
        )?);
        definition.response_fields_for_selection(
            &options.target_lang,
            &context,
            &selection,
            &prefix,
        )?
    };
    let variables_struct =
        operation.expand_variables(&context, &options.target_lang, &operation.operation_type);

    let input_object_definitions: Result<Vec<TokenStream>, _> = context
        .schema
        .inputs
        .values()
        .filter_map(|i| {
            if i.is_required.get() {
                Some(match options.target_lang {
                    TargetLang::Rust => i.to_rust(&context),
                    TargetLang::Go => i.to_go(&context),
                })
            } else {
                None
            }
        })
        .collect();
    let input_object_definitions = input_object_definitions?;

    let fragment_definitions: Result<Vec<TokenStream>, _> = context
        .fragments
        .values()
        .filter_map(|fragment| {
            if fragment.is_required.get() {
                Some(match options.target_lang {
                    TargetLang::Rust => fragment.to_rust(&context),
                    TargetLang::Go => fragment.to_go(&context),
                })
            } else {
                None
            }
        })
        .collect();
    let fragment_definitions = fragment_definitions?;

    let scalar_definitions: Vec<TokenStream> = context
        .schema
        .scalars
        .values()
        .filter_map(|s| {
            if s.is_required.get() {
                Some(match options.target_lang {
                    TargetLang::Rust => s.to_rust(),
                    TargetLang::Go => s.to_go(),
                })
            } else {
                None
            }
        })
        .collect();

    let response_derives = context.response_derives();

    let enum_definitions: Vec<TokenStream> = context
        .schema
        .enums
        .values()
        .filter_map(|enm| {
            if enm.is_required.get() {
                Some(match options.target_lang {
                    TargetLang::Rust => enm.to_rust(&context),
                    TargetLang::Go => enm.to_go(&context),
                })
            } else {
                None
            }
        })
        .collect();

    match options.target_lang {
        TargetLang::Rust => Ok(quote! {
            use serde::{Serialize, Deserialize};

            #[allow(dead_code)]
            type Boolean = bool;
            #[allow(dead_code)]
            type Float = f64;
            #[allow(dead_code)]
            type Int = i64;
            #[allow(dead_code)]
            type ID = String;

            #(#scalar_definitions)*

            #(#input_object_definitions)*

            #(#enum_definitions)*

            #(#fragment_definitions)*

            #(#definitions)*

            #variables_struct

            #response_derives

            pub struct ResponseData {
                #(#response_data_fields,)*
            }
        }),
        TargetLang::Go => Ok(quote! {
            type Boolean = bool;
            type Float = float64;
            type Int = int64;
            type String = string;
            type ID = string;

            #(#scalar_definitions;)*

            #(#input_object_definitions;)*

            #(#enum_definitions;)*

            #(#fragment_definitions;)*

            #(#definitions;)*

            #variables_struct;

            type ResponseData struct {
                #(#response_data_fields;)*
            };
            func (resp *ResponseData) UnmarshalGQL(buf []byte) error {
                return errors.WithStack(resp.UnmarshalJSON(buf));
            };
        }),
    }
}
