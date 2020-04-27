#![recursion_limit = "128"]
#![deny(rust_2018_idioms)]

//! Crate for internal use by other graphql-client crates, for code generation.
//!
//! It is not meant to be used directly by users of the library.

use failure::*;
use lazy_static::*;
use proc_macro2::TokenStream;
use quote::*;

mod codegen;
mod codegen_options;
/// Deprecation-related code
pub mod deprecation;
mod query;
/// Contains the [Schema] type and its implementation.
pub mod schema;

mod constants;
mod enums;
mod field_type;
mod fragments;
mod generated_module;
mod inputs;
mod interfaces;
mod objects;
mod operations;
mod scalars;
mod selection;
mod shared;
mod unions;
mod variables;

#[cfg(test)]
mod tests;

pub use crate::codegen_options::{CodegenMode, GraphQLClientCodegenOptions, TargetLang};

use std::collections::HashMap;

type CacheMap<T> = std::sync::Mutex<HashMap<std::path::PathBuf, T>>;

lazy_static! {
    static ref SCHEMA_CACHE: CacheMap<String> = CacheMap::default();
    static ref QUERY_CACHE: CacheMap<(String, graphql_parser::query::Document)> =
        CacheMap::default();
}

/// Generates code given a query document, a schema and options.
pub fn generate_module_token_stream(
    target_lang: &TargetLang,
    query_path: std::path::PathBuf,
    schema_path: &std::path::Path,
    options: GraphQLClientCodegenOptions,
) -> Result<TokenStream, failure::Error> {
    use std::collections::hash_map;
    // We need to qualify the query with the path to the crate it is part of
    let (query_string, query) = {
        let mut lock = QUERY_CACHE.lock().expect("query cache is poisoned");
        match lock.entry(query_path) {
            hash_map::Entry::Occupied(o) => o.get().clone(),
            hash_map::Entry::Vacant(v) => {
                let query_string = read_file(v.key())?;
                let query = graphql_parser::parse_query(&query_string)?;

                v.insert((query_string, query)).clone()
            }
        }
    };

    // Determine which operation we are generating code for. This will be used in
    // operationName.
    let operations = options
        .operation_name
        .as_ref()
        .and_then(|operation_name| codegen::select_operation(&query, &operation_name))
        .map(|op| vec![op]);

    let operations = match (operations, &options.mode) {
        (Some(ops), _) => ops,
        (None, &CodegenMode::Cli) => codegen::all_operations(&query),
        (None, &CodegenMode::Derive) => {
            return Err(derive_operation_not_found_error(
                options.struct_ident(),
                &query,
            ));
        }
    };

    let schema_extension = schema_path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("INVALID");

    // Check the schema cache.
    let schema_string: String = {
        let mut lock = SCHEMA_CACHE.lock().expect("schema cache is poisoned");
        match lock.entry(schema_path.to_path_buf()) {
            hash_map::Entry::Occupied(o) => o.get().clone(),
            hash_map::Entry::Vacant(v) => {
                let schema_string = read_file(v.key())?;
                v.insert(schema_string).to_string()
            }
        }
    };

    let parsed_schema = match schema_extension {
                        "graphql" | "gql" => {
                            let s = graphql_parser::schema::parse_schema(&schema_string)?;
                            schema::ParsedSchema::GraphQLParser(s)
                        }
                        "json" => {
                            let parsed: graphql_introspection_query::introspection_response::IntrospectionResponse = serde_json::from_str(&schema_string)?;
                            schema::ParsedSchema::Json(parsed)
                        }
                        extension => panic!("Unsupported extension for the GraphQL schema: {} (only .json and .graphql are supported)", extension)
                    };

    let schema = schema::Schema::from(&parsed_schema);

    for p in &query.definitions {
        use graphql_parser::query::{
            Definition::Operation,
            OperationDefinition::{Mutation, Query, Subscription},
        };
        let variable_definitions: &[_] = match p {
            Operation(Query(q)) => &q.variable_definitions,
            Operation(Mutation(m)) => &m.variable_definitions,
            Operation(Subscription(s)) => &s.variable_definitions,
            _ => &[],
        };
        for v in variable_definitions {
            let mut t = v.var_type.clone();
            loop {
                use graphql_parser::query::Type;
                match t {
                    Type::NamedType(name) => {
                        schema
                            .enums
                            .get(&*name)
                            .map(|enm| enm.is_required.set(true));
                        break;
                    }
                    Type::ListType(v) => t = *v,
                    Type::NonNullType(v) => t = *v,
                }
            }
        }
    }
    // The generated modules.
    let mut modules = Vec::with_capacity(operations.len());

    for operation in &operations {
        let generated = generated_module::GeneratedModule {
            query_string: query_string.as_str(),
            schema: &schema,
            query_document: &query,
            operation,
            options: &options,
        }
        .to_token_stream(target_lang)?;
        modules.push(generated);
    }

    let modules = quote! { #(#modules)* };

    Ok(modules)
}

fn read_file(path: &std::path::Path) -> Result<String, failure::Error> {
    use std::{fs, io::prelude::*};

    let mut out = String::new();
    let mut file = fs::File::open(path).map_err(|io_err| {
        let err: failure::Error = io_err.into();
        err.context(format!(
            r#"
            Could not find file with path: {}
            Hint: file paths in the GraphQLQuery attribute are relative to the project root (location of the Cargo.toml). Example: query_path = "src/my_query.graphql".
            "#,
            path.display()
        ))
    })?;
    file.read_to_string(&mut out)?;
    Ok(out)
}

/// In derive mode, build an error when the operation with the same name as the
/// struct is not found.
fn derive_operation_not_found_error(
    ident: Option<&proc_macro2::Ident>,
    query: &graphql_parser::query::Document,
) -> failure::Error {
    use graphql_parser::query::*;

    let operation_name = ident.map(ToString::to_string);
    let struct_ident = operation_name.as_ref().map(String::as_str).unwrap_or("");

    let available_operations = query
        .definitions
        .iter()
        .filter_map(|definition| match definition {
            Definition::Operation(op) => match op {
                OperationDefinition::Mutation(m) => Some(m.name.as_ref().unwrap()),
                OperationDefinition::Query(m) => Some(m.name.as_ref().unwrap()),
                OperationDefinition::Subscription(m) => Some(m.name.as_ref().unwrap()),
                OperationDefinition::SelectionSet(_) => {
                    unreachable!("Bare selection sets are not supported.")
                }
            },
            _ => None,
        })
        .fold(String::new(), |mut acc, item| {
            acc.push_str(&item);
            acc.push_str(", ");
            acc
        });

    let available_operations = available_operations.trim_end_matches(", ");

    return format_err!(
        "The struct name does not match any defined operation in the query file.\nStruct name: {}\nDefined operations: {}",
        struct_ident,
        available_operations,
    );
}
