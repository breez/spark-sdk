use crate::ssp::graphql::{
    BitcoinNetwork, ClaimStaticDepositRequestType, CurrencyUnit, ExitSpeed,
    LightningReceiveRequestStatus, LightningSendRequestStatus, SparkCoopExitRequestStatus,
    SparkLeavesSwapRequestStatus,
};
use graphql_client::GraphQLQuery;

// Define the types used as scalar types in the GraphQL schema
type PublicKey = String;
type UUID = String;
type DateTime = chrono::DateTime<chrono::Utc>;
type Hash32 = String;
type Long = u64;

// Define fragmented types for GraphQL queries

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums("BitcoinNetwork", "ClaimStaticDepositRequestType")
)]
pub struct ClaimStaticDeposit;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums(
        "CurrencyUnit",
        "ExitSpeed",
        "BitcoinNetwork",
        "SparkCoopExitRequestStatus"
    )
)]
pub struct CompleteCoopExit;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums("CurrencyUnit", "BitcoinNetwork", "SparkLeavesSwapRequestStatus")
)]
pub struct CompleteLeavesSwap;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums("CurrencyUnit")
)]
pub struct CoopExitFeeEstimates;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug"
)]
pub struct GetChallenge;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums("CurrencyUnit")
)]
pub struct LeavesSwapFeeEstimate;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums("CurrencyUnit")
)]
pub struct LightningSendFeeEstimate;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums(
        "CurrencyUnit",
        "ExitSpeed",
        "BitcoinNetwork",
        "SparkCoopExitRequestStatus"
    )
)]
pub struct RequestCoopExit;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums("CurrencyUnit", "BitcoinNetwork", "SparkLeavesSwapRequestStatus")
)]
pub struct RequestLeavesSwap;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums(
        "CurrencyUnit",
        "ExitSpeed",
        "BitcoinNetwork",
        "LightningReceiveRequestStatus"
    )
)]
pub struct RequestLightningReceive;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums("CurrencyUnit", "BitcoinNetwork", "LightningSendRequestStatus")
)]
pub struct RequestLightningSend;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums("BitcoinNetwork")
)]
pub struct StaticDepositQuote;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums("CurrencyUnit")
)]
pub struct Transfer;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug",
    extern_enums(
        "CurrencyUnit",
        "ExitSpeed",
        "BitcoinNetwork",
        "LightningReceiveRequestStatus",
        "LightningSendRequestStatus",
        "SparkCoopExitRequestStatus",
        "SparkLeavesSwapRequestStatus"
    )
)]
pub struct UserRequest;

#[derive(GraphQLQuery)]
#[graphql(
    query_path = "schema/queries.graphql",
    schema_path = "schema/spark.graphql",
    variables_derives = "Clone",
    response_derives = "Debug"
)]
pub struct VerifyChallenge;
