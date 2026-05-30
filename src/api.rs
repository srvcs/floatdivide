use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{OpenApi, ToSchema};

use crate::client::{self, DepError};

pub const SERVICE: &str = "srvcs-floatdivide";
pub const CONCERN: &str = "float arithmetic: a / b";
pub const DEPENDS_ON: &[&str] = &["srvcs-isnumber"];

/// Dependency endpoints, injected as router state so tests can point them at
/// mock services.
#[derive(Clone)]
pub struct Deps {
    pub isnumber_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct Info {
    pub service: &'static str,
    pub concern: &'static str,
    pub depends_on: Vec<&'static str>,
}

/// `GET /` — service identity (srvcs service standard).
#[utoipa::path(get, path = "/", responses((status = 200, body = Info)))]
pub async fn index() -> Json<Info> {
    Json(Info {
        service: SERVICE,
        concern: CONCERN,
        depends_on: DEPENDS_ON.to_vec(),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct EvalRequest {
    #[schema(value_type = Object)]
    pub a: Value,
    #[schema(value_type = Object)]
    pub b: Value,
}

#[derive(Serialize, ToSchema)]
pub struct ResultResponse {
    #[schema(value_type = Object)]
    pub a: Value,
    #[schema(value_type = Object)]
    pub b: Value,
    /// The quotient `a / b` as a floating-point number.
    pub result: f64,
}

/// The single concern: the floating-point quotient of two real numbers.
///
/// This is a pure `f64` division; callers are responsible for rejecting a zero
/// divisor before calling (see the `422` path in [`evaluate`]).
pub fn float_divide(a: f64, b: f64) -> f64 {
    a / b
}

fn ok(a: Value, b: Value, result: f64) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "a": a, "b": b, "result": result })),
    )
        .into_response()
}

fn invalid(reason: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": reason })),
    )
        .into_response()
}

fn degraded(dependency: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "dependency unavailable", "dependency": dependency })),
    )
        .into_response()
}

/// Forward a dependency's response verbatim (used to propagate `422` for invalid
/// input, so floatdivide reports the same rejection its dependency did).
fn forward(status: u16, body: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(body)).into_response()
}

/// Validate `value` is a number by asking `srvcs-isnumber`, mapping its
/// failures to the response this service should return.
async fn ask_is_number(url: &str, value: &Value, dependency: &str) -> Result<(), Response> {
    match client::call(url, &json!({ "value": value })).await {
        Err(DepError::Unreachable) => Err(degraded(dependency)),
        Ok((200, body)) => {
            let is_number = body.get("result").and_then(Value::as_bool).unwrap_or(false);
            if is_number {
                Ok(())
            } else {
                Err(invalid("value is not a number"))
            }
        }
        // Invalid input propagates from the leaf dependency; forward it.
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded(dependency)),
    }
}

/// `POST /` — compute `a / b` as a floating-point quotient.
///
/// Input validation is delegated to `srvcs-isnumber` over HTTP (the single
/// source of truth for "is this a number"), once per operand. Both integers and
/// floats are valid operands — they are coerced with `.as_f64()`. A zero divisor
/// is a domain error (`422 division by zero`). If the dependency is unreachable,
/// this service reports itself degraded rather than guessing.
#[utoipa::path(
    post,
    path = "/",
    request_body = EvalRequest,
    responses(
        (status = 200, body = ResultResponse),
        (status = 422, description = "an operand is not a number, or b is zero (division by zero)"),
        (status = 503, description = "a dependency is unavailable")
    )
)]
pub async fn evaluate(State(deps): State<Deps>, Json(req): Json<EvalRequest>) -> Response {
    // 1. Delegate "is this a number" to srvcs-isnumber, once per operand.
    if let Err(resp) = ask_is_number(&deps.isnumber_url, &req.a, "srvcs-isnumber").await {
        return resp;
    }
    if let Err(resp) = ask_is_number(&deps.isnumber_url, &req.b, "srvcs-isnumber").await {
        return resp;
    }

    // 2. Coerce both operands to f64 (accepts both integers and floats).
    let Some(a) = req.a.as_f64() else {
        return invalid("a validated as a number but is not representable as f64");
    };
    let Some(b) = req.b.as_f64() else {
        return invalid("b validated as a number but is not representable as f64");
    };

    // 3. Division by zero is a domain error.
    if b == 0.0 {
        return invalid("division by zero");
    }

    ok(req.a, req.b, float_divide(a, b))
}

#[derive(OpenApi)]
#[openapi(
    paths(index, evaluate),
    components(schemas(Info, EvalRequest, ResultResponse))
)]
pub struct ApiDoc;

/// Serve OpenAPI document
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_documents_routes() {
        let doc = ApiDoc::openapi();
        let root = doc.paths.paths.get("/").expect("path / present");
        assert!(root.get.is_some());
        assert!(root.post.is_some());
    }

    #[test]
    fn divides_floats() {
        assert!((float_divide(7.0, 2.0) - 3.5).abs() < 1e-9);
        assert!((float_divide(1.0, 4.0) - 0.25).abs() < 1e-9);
        assert!((float_divide(-9.0, 3.0) - -3.0).abs() < 1e-9);
        assert!((float_divide(0.0, 5.0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn divides_integers_as_floats() {
        // Integer operands still produce a fractional quotient.
        assert!((float_divide(5.0, 2.0) - 2.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn index_reports_dependency() {
        let Json(info) = index().await;
        assert_eq!(info.service, "srvcs-floatdivide");
        assert_eq!(info.concern, "float arithmetic: a / b");
        assert_eq!(info.depends_on, vec!["srvcs-isnumber"]);
    }
}
