use crate::{
    constants::*, query::QueryContext, selection::Selection, variables::Variable, TargetLang,
};
use graphql_parser::query::OperationDefinition;
use heck::{CamelCase, SnakeCase};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Ident;

#[derive(Debug, Clone)]
pub enum OperationType {
    Query,
    Mutation,
    Subscription,
}

#[derive(Debug, Clone)]
pub struct Operation<'query> {
    pub name: String,
    pub operation_type: OperationType,
    pub variables: Vec<Variable<'query>>,
    pub selection: Selection<'query>,
}

impl<'query> Operation<'query> {
    pub(crate) fn root_name<'schema>(
        &self,
        schema: &'schema crate::schema::Schema<'_>,
    ) -> &'schema str {
        match self.operation_type {
            OperationType::Query => schema.query_type.unwrap_or("Query"),
            OperationType::Mutation => schema.mutation_type.unwrap_or("Mutation"),
            OperationType::Subscription => schema.subscription_type.unwrap_or("Subscription"),
        }
    }

    pub(crate) fn is_subscription(&self) -> bool {
        match self.operation_type {
            OperationType::Subscription => true,
            _ => false,
        }
    }

    /// Generate the Variables struct and all the necessary supporting code.
    pub(crate) fn expand_variables(
        &self,
        context: &QueryContext<'_, '_>,
        target_lang: &TargetLang,
        operation_type: &OperationType,
    ) -> TokenStream {
        let variables = &self.variables;
        let variables_derives = context.variables_derives();

        if variables.is_empty() {
            match target_lang {
                TargetLang::Rust => {
                    return quote! {
                        #variables_derives
                        pub struct Variables;
                    };
                }
                TargetLang::Go => {
                    return quote! {
                        type Variables struct{}
                    };
                }
            }
        }

        let fields: Vec<TokenStream> = variables
            .iter()
            .map(|variable| {
                let ty = match target_lang {
                    TargetLang::Rust => variable.ty.to_rust(context, ""),
                    TargetLang::Go => variable.ty.to_go(context, ""),
                };
                let name = match target_lang {
                    TargetLang::Rust => {
                        crate::shared::keyword_replace(&variable.name.to_snake_case())
                    }
                    TargetLang::Go => variable.name.to_camel_case(),
                };

                let rename = crate::shared::field_rename_annotation(&variable.name, &name);
                let name = Ident::new(&name, Span::call_site());

                let raw_name = Ident::new(&variable.name, Span::call_site());
                match target_lang {
                    TargetLang::Rust => quote!(#rename pub #name: #ty),
                    TargetLang::Go => match operation_type {
                        OperationType::Mutation => {
                            quote!(#name #ty __JSON_TAGS_WITHOUT_OMIT(#raw_name))
                        }
                        _ => quote!(#name #ty __JSON_TAGS(#raw_name)),
                    },
                }
            })
            .collect();

        let default_constructors = variables
            .iter()
            .map(|variable| variable.generate_default_value_constructor(context, target_lang));

        match target_lang {
            TargetLang::Rust => quote! {
                #variables_derives
                pub struct Variables {
                    #(#fields,)*
                }

                impl Variables {
                    #(#default_constructors)*
                }
            },
            TargetLang::Go => quote! {
                #(#default_constructors)*

                type Variables struct {
                    #(#fields;)*
                }
            },
        }
    }
}

impl<'query> std::convert::From<&'query OperationDefinition> for Operation<'query> {
    fn from(definition: &'query OperationDefinition) -> Operation<'query> {
        match *definition {
            OperationDefinition::Query(ref q) => Operation {
                name: q.name.clone().expect("unnamed operation"),
                operation_type: OperationType::Query,
                variables: q.variable_definitions.iter().map(|v| v.into()).collect(),
                selection: (&q.selection_set).into(),
            },
            OperationDefinition::Mutation(ref m) => Operation {
                name: m.name.clone().expect("unnamed operation"),
                operation_type: OperationType::Mutation,
                variables: m.variable_definitions.iter().map(|v| v.into()).collect(),
                selection: (&m.selection_set).into(),
            },
            OperationDefinition::Subscription(ref s) => Operation {
                name: s.name.clone().expect("unnamed operation"),
                operation_type: OperationType::Subscription,
                variables: s.variable_definitions.iter().map(|v| v.into()).collect(),
                selection: (&s.selection_set).into(),
            },
            OperationDefinition::SelectionSet(_) => panic!(SELECTION_SET_AT_ROOT),
        }
    }
}
