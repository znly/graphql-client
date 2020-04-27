use crate::{codegen_options::*, TargetLang};
use heck::*;
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;

/// This struct contains the parameters necessary to generate code for a given
/// operation.
pub(crate) struct GeneratedModule<'a> {
    pub operation: &'a crate::operations::Operation<'a>,
    pub query_string: &'a str,
    pub query_document: &'a graphql_parser::query::Document,
    pub schema: &'a crate::schema::Schema<'a>,
    pub options: &'a crate::GraphQLClientCodegenOptions,
}

impl<'a> GeneratedModule<'a> {
    /// Generate the items for the variables and the response that will go
    /// inside the module.
    fn build_impls(&self) -> Result<TokenStream, failure::Error> {
        Ok(crate::codegen::response_for_query(
            &self.schema,
            &self.query_document,
            &self.operation,
            &self.options,
        )?)
    }

    /// Generate the module and all the code inside.
    pub(crate) fn to_token_stream(
        &self,
        target_lang: &TargetLang,
    ) -> Result<TokenStream, failure::Error> {
        let module_name = Ident::new(&self.operation.name.to_snake_case(), Span::call_site());
        let module_visibility = &self.options.module_visibility();
        let operation_name_literal = &self.operation.name;
        let operation_name_ident = operation_name_literal.clone();
        #[cfg(feature = "normalize_query_types")]
        let operation_name_ident =
            if Some(operation_name_ident.to_camel_case()) == self.options.operation_name {
                operation_name_ident.to_camel_case()
            } else {
                operation_name_ident
            };
        let operation_name_ident = Ident::new(&operation_name_ident, Span::call_site());

        // Force cargo to refresh the generated code when the query file changes.
        let query_include = self
            .options
            .query_file()
            .map(|path| {
                let path = path.to_str();
                quote!(
                    const __QUERY_WORKAROUND: &str = include_str!(#path);
                )
            })
            .unwrap_or_else(|| quote! {});

        let query_string = &self.query_string;
        let impls = self.build_impls()?;

        let struct_declaration: Option<_> = match self.options.mode {
            CodegenMode::Cli => match target_lang {
                TargetLang::Rust => Some(quote!(#module_visibility struct #operation_name_ident;)),
                TargetLang::Go => Some(quote!(type #operation_name_ident struct{})),
            },

            // The struct is already present in derive mode.
            CodegenMode::Derive => None,
        };

        match target_lang {
            TargetLang::Rust => Ok(quote!(
                #struct_declaration

                #module_visibility mod #module_name {
                    #![allow(dead_code)]

                    pub const OPERATION_NAME: &'static str = #operation_name_literal;
                    pub const QUERY: &'static str = #query_string;

                    #query_include

                    #impls
                }

                impl graphql_client::GraphQLQuery for #operation_name_ident {
                    type Variables = #module_name::Variables;
                    type ResponseData = #module_name::ResponseData;

                    fn build_query(variables: Self::Variables) -> ::graphql_client::QueryBody<Self::Variables> {
                        graphql_client::QueryBody {
                            variables,
                            query: #module_name::QUERY,
                            operation_name: #module_name::OPERATION_NAME,
                        }

                    }
                }
            )),
            TargetLang::Go => Ok(quote!(
                #query_include

                package #module_name;

                #struct_declaration;

                #impls

                type Query struct {
                    Vars Variables __JSON_TAGS(variables);
                    Query string __JSON_TAGS(query);
                    OperationName string __JSON_TAGS(operationName);
                };
                func (q *Query) MarshalGQL() (buf []byte, err error) {
                    buf, err = json.Marshal(q);
                    return buf, errors.WithStack(err);
                };

                func NewQuery(vars Variables) Query {
                    const OPERATION_NAME = #operation_name_literal;
                    const QUERY = #query_string;
                    return Query {
                        Vars: vars,
                        Query: QUERY,
                        OperationName: OPERATION_NAME,
                    };
                };
            )),
        }
    }
}
