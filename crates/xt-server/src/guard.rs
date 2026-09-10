use crate::AppState;
use axum::{
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::Next,
    response::Response,
};

fn single(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    let mut values = headers.get_all(name).iter();
    let first = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(first)
}
fn authority(value: &str, port: u16) -> bool {
    [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ]
    .iter()
    .any(|expected| value.eq_ignore_ascii_case(expected))
}

pub(crate) async fn loopback(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let headers = request.headers();
    let host = single(headers, header::HOST)
        .filter(|host| authority(host, state.port))
        .ok_or(StatusCode::FORBIDDEN)?;
    if headers.contains_key(header::ORIGIN) {
        let origin = single(headers, header::ORIGIN).ok_or(StatusCode::FORBIDDEN)?;
        let expected = format!("http://{host}");
        if !origin.eq_ignore_ascii_case(&expected) {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    if request.method() == Method::POST {
        let content =
            single(headers, header::CONTENT_TYPE).ok_or(StatusCode::UNSUPPORTED_MEDIA_TYPE)?;
        if !content
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
        {
            return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
        }
    }
    Ok(next.run(request).await)
}
