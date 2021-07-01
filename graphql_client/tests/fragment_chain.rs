use graphql_client::*;

#[allow(dead_code)]
#[derive(GraphQLQuery)]
#[graphql(
    normalization = "rust",
    schema_path = "tests/fragment_chain/schema.graphql",
    query_path = "tests/fragment_chain/query.graphql"
)]
struct Q;
