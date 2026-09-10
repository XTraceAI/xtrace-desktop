use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

fn envelope(data: Value) -> Json<Value> {
    Json(json!({"code":0,"msg":"ok","data":data}))
}
fn token(label: &str, scopes: Value, expires: Value) -> Value {
    let now: chrono::DateTime<chrono::Utc> = std::time::SystemTime::now().into();
    json!({"id":"local","label":label,"scopes":scopes,"expires_at":expires,"created_at":now.to_rfc3339_opts(chrono::SecondsFormat::Secs,true)})
}
#[derive(Deserialize)]
pub(crate) struct Mint {
    label: String,
    scopes: Vec<String>,
    expires_at: Option<String>,
}
pub(crate) async fn mint(Json(input): Json<Mint>) -> Json<Value> {
    envelope(
        json!({"secret":"local","access_token":token(&input.label,json!(input.scopes),json!(input.expires_at))}),
    )
}
pub(crate) async fn tokens() -> Json<Value> {
    envelope(json!([token(
        "Local desktop",
        json!(["memory:read", "memory:write"]),
        Value::Null
    )]))
}
pub(crate) async fn revoke() -> Json<Value> {
    envelope(Value::Null)
}

fn sse(value: Value) -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        format!("event: message\ndata: {value}\n\n"),
    )
        .into_response()
}
fn error(id: Value, code: i32, message: &str) -> Response {
    sse(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
}
pub(crate) async fn mcp(
    body: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(input) = match body {
        Ok(body) => body,
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
        Err(_) => return error(Value::Null, -32700, "Parse error"),
    };
    let id = input.get("id").cloned().unwrap_or(Value::Null);
    if !input.is_object()
        || input.get("jsonrpc") != Some(&json!("2.0"))
        || !input.get("method").is_some_and(Value::is_string)
        || !(id.is_null() || id.is_string() || id.is_number())
    {
        return error(Value::Null, -32600, "Invalid Request");
    }
    if !input
        .get("params")
        .is_none_or(|params| params.is_object() || params.is_array())
    {
        return error(id, -32602, "Invalid params");
    }
    let method = input["method"].as_str().unwrap();
    if !input.as_object().unwrap().contains_key("id") {
        return StatusCode::ACCEPTED.into_response();
    }
    let result = match method {
        "initialize" => {
            json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"xtrace-core","version":env!("CARGO_PKG_VERSION")}})
        }
        "tools/list" => {
            json!({"tools":[{"name":"import_conversation","description":"Import a coding session (not implemented in this transport scaffold).","inputSchema":{"type":"object","properties":{"conversation_id":{"type":"string"},"messages":{"type":"array","items":{"type":"object"}},"source_platform":{"type":"string"}},"required":["conversation_id","messages","source_platform"]}}]})
        }
        "tools/call" if input["params"]["name"] == "import_conversation" => {
            json!({"isError":true,"content":[{"type":"text","text":"Conversation import is not implemented."}]})
        }
        "tools/call" => return error(id, -32602, "Unknown tool"),
        "ping" => json!({}),
        _ => return error(id, -32601, "Method not found"),
    };
    sse(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
