//! Starlette's route *order*, which axum doesn't have.
//!
//! Starlette tries the routes one by one in registration order on the
//! percent-decoded path: the first route matching path **and** method
//! handles the request; failing that, the first route matching the path
//! alone answers 405 with *its* methods in `Allow` (one method: FastAPI
//! registers one route per decorator). axum instead prefers static segments
//! and lists every method of the path. The two disagree when a static path
//! collides with a sibling `{id}` route: `PATCH /api/transactions/years` is
//! handled by `PATCH /api/transactions/{transaction_id}` in FastAPI (→ 422,
//! `int_parsing` of `"years"`, together with any body errors) but would be
//! a 405 in axum.
//!
//! [`resolve`] replays Starlette's decision over [`ROUTES`] (FastAPI's
//! registration order: `main.py`'s `include_router` order, then decorator
//! order inside each router). The routing middleware then either answers
//! the 405 itself or, when axum would pick a different route than
//! Starlette, rewrites the path so that axum lands on Starlette's route: the
//! parameter segments are percent-encoded (their first byte always), which
//! no static segment matches but axum's path extractors decode back to the
//! original value. A path with percent-escapes is rewritten the same way,
//! so `/api/tag%73` routes like `/api/tags`, as in Starlette.
//!
//! **Adding a route:** add it to [`ROUTES`] at the position FastAPI
//! registers it (a unit test checks the table against the router).

/// `(method, path)` of every API route in FastAPI's registration order.
/// The docs pages are plain Starlette routes outside this table.
pub const ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/dashboard/summary"),
    ("GET", "/api/accounts"),
    ("POST", "/api/accounts"),
    ("PATCH", "/api/accounts/{account_id}"),
    ("DELETE", "/api/accounts/{account_id}"),
    ("GET", "/api/categories"),
    ("POST", "/api/categories"),
    ("PATCH", "/api/categories/{category_id}"),
    ("DELETE", "/api/categories/{category_id}"),
    ("GET", "/api/transactions"),
    ("GET", "/api/transactions/years"),
    ("POST", "/api/transactions"),
    ("POST", "/api/transactions/bulk"),
    ("PATCH", "/api/transactions/{transaction_id}"),
    ("DELETE", "/api/transactions/{transaction_id}"),
    ("GET", "/api/assets"),
    ("POST", "/api/assets"),
    ("PATCH", "/api/assets/{asset_id}"),
    ("POST", "/api/assets/{asset_id}/valuations"),
    ("GET", "/api/assets/{asset_id}/valuations"),
    ("DELETE", "/api/assets/{asset_id}"),
    ("GET", "/api/net-worth/summary"),
    ("GET", "/api/backup/export"),
    ("POST", "/api/backup/import"),
    ("GET", "/api/reports/category-spending"),
    ("GET", "/api/reports/category-ranking"),
    ("GET", "/api/insights/alerts"),
    ("GET", "/api/settings"),
    ("PATCH", "/api/settings"),
    ("GET", "/api/budgets"),
    ("GET", "/api/budgets/status"),
    ("POST", "/api/budgets"),
    ("PATCH", "/api/budgets/{budget_id}"),
    ("DELETE", "/api/budgets/{budget_id}"),
    ("GET", "/api/advice"),
    ("GET", "/api/goals"),
    ("POST", "/api/goals"),
    ("PATCH", "/api/goals/{goal_id}"),
    ("DELETE", "/api/goals/{goal_id}"),
    ("POST", "/api/goals/{goal_id}/contributions"),
    ("GET", "/api/recurring"),
    ("POST", "/api/recurring"),
    ("PATCH", "/api/recurring/{recurring_id}"),
    ("DELETE", "/api/recurring/{recurring_id}"),
    ("POST", "/api/recurring/{recurring_id}/post"),
    ("GET", "/api/cash-flow"),
    ("GET", "/api/tags"),
    ("POST", "/api/tags"),
    ("DELETE", "/api/tags/{tag_id}"),
    ("GET", "/api/crypto/portfolios"),
    ("POST", "/api/crypto/portfolios"),
    ("PATCH", "/api/crypto/portfolios/{portfolio_id}"),
    ("DELETE", "/api/crypto/portfolios/{portfolio_id}"),
    ("GET", "/api/crypto/holdings"),
    ("POST", "/api/crypto/refresh"),
    ("POST", "/api/crypto/holdings"),
    ("PATCH", "/api/crypto/holdings/{asset_id}"),
    ("POST", "/api/crypto/holdings/{asset_id}/transactions"),
    ("GET", "/api/crypto/holdings/{asset_id}/transactions"),
    ("PATCH", "/api/crypto/transactions/{transaction_id}"),
    ("DELETE", "/api/crypto/transactions/{transaction_id}"),
    ("GET", "/api/crypto/search"),
    ("GET", "/api/crypto/history"),
    ("GET", "/api/crypto/performance/90d"),
    ("GET", "/api/health"),
];

/// Starlette's decision for one request.
#[derive(Debug, PartialEq, Eq)]
pub enum Resolution {
    /// No route matches the path: 404 / slash redirect / docs (left to
    /// axum). `rewrite` is the decoded path when escapes changed it, so that
    /// axum's 404/redirect logic sees what Starlette sees (`/api/tags/1%2F2`
    /// is `/api/tags/1/2`, a 404).
    NoMatch { rewrite: Option<String> },
    /// The path matches but no route has the method: 405, `Allow: <allow>`.
    MethodNotAllowed { allow: &'static str },
    /// A route handles it. `rewrite` is the path axum must see to dispatch
    /// to that same route (`None` = the request path already does).
    Handle { rewrite: Option<String> },
}

/// Replays Starlette's routing of `method raw_path` over [`ROUTES`].
pub fn resolve(method: &str, raw_path: &str) -> Resolution {
    // uvicorn hands Starlette the percent-decoded path.
    let decoded = percent_decode(raw_path);
    let segments: Vec<&str> = decoded.split('/').collect();
    let mut first_partial = None;
    let mut full = None;
    for (i, (route_method, template)) in ROUTES.iter().enumerate() {
        if !matches(template, &segments) {
            continue;
        }
        if *route_method == method {
            full = Some(i);
            break;
        }
        first_partial.get_or_insert(*route_method);
    }
    let Some(i) = full else {
        return match first_partial {
            Some(allow) => Resolution::MethodNotAllowed { allow },
            None if decoded == raw_path => Resolution::NoMatch { rewrite: None },
            None => Resolution::NoMatch { rewrite: Some(encode_path(&decoded)) },
        };
    };
    let template = ROUTES[i].1;
    // axum agrees unless escapes changed the path or another matching
    // template is more static (axum prefers a literal segment over a
    // parameter at the first position where two templates differ).
    let more_static = |other: &str| {
        other.split('/').zip(template.split('/')).find(|(a, b)| a != b).is_some_and(|(a, _)| !is_param(a))
    };
    let shadowed = ROUTES.iter().any(|(_, other)| matches(other, &segments) && more_static(other));
    if decoded == raw_path && !shadowed {
        return Resolution::Handle { rewrite: None };
    }
    let rewritten: Vec<String> = template
        .split('/')
        .zip(&segments)
        .map(|(part, value)| if is_param(part) { encode_param(value) } else { part.to_string() })
        .collect();
    Resolution::Handle { rewrite: Some(rewritten.join("/")) }
}

fn is_param(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}')
}

/// Starlette's `[^/]+` for a parameter, equality for a literal.
fn matches(template: &str, segments: &[&str]) -> bool {
    let parts: Vec<&str> = template.split('/').collect();
    parts.len() == segments.len()
        && parts
            .iter()
            .zip(segments)
            .all(|(part, value)| if is_param(part) { !value.is_empty() } else { part == value })
}

/// `urllib.parse.unquote` (invalid UTF-8 → U+FFFD, malformed escapes kept).
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(&h), Some(&l)) = (bytes.get(i + 1), bytes.get(i + 2))
            && let (Some(h), Some(l)) = (hex(h), hex(l))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Re-encodes a decoded path for the URI, keeping its `/`s.
fn encode_path(decoded: &str) -> String {
    let mut out = String::with_capacity(decoded.len());
    for byte in decoded.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/!$&'()*+,;=:@".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Percent-encodes a parameter value so that it matches no static segment
/// (the first byte is always escaped) and decodes back to `value`.
fn encode_param(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 3);
    for (i, byte) in value.bytes().enumerate() {
        if i > 0 && (byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_lists_exactly_the_routers_routes() {
        let (_, openapi) = crate::routes::api_router().split_for_parts();
        let mut from_router: Vec<(String, String)> = Vec::new();
        for (path, item) in &openapi.paths.paths {
            let ops = [
                ("GET", &item.get),
                ("POST", &item.post),
                ("PUT", &item.put),
                ("PATCH", &item.patch),
                ("DELETE", &item.delete),
                ("HEAD", &item.head),
                ("OPTIONS", &item.options),
            ];
            for (method, op) in ops {
                if op.is_some() {
                    from_router.push((method.to_string(), path.clone()));
                }
            }
        }
        let mut from_table: Vec<(String, String)> =
            ROUTES.iter().map(|(m, p)| (m.to_string(), p.to_string())).collect();
        from_router.sort();
        from_table.sort();
        assert_eq!(from_table, from_router);
    }

    #[test]
    fn replays_starlette_order() {
        let rewrite = |r: &str| Resolution::Handle { rewrite: Some(r.to_string()) };
        assert_eq!(resolve("GET", "/api/transactions/years"), Resolution::Handle { rewrite: None });
        assert_eq!(resolve("PATCH", "/api/transactions/years"), rewrite("/api/transactions/%79ears"));
        assert_eq!(resolve("PATCH", "/api/transactions/12"), Resolution::Handle { rewrite: None });
        assert_eq!(resolve("PUT", "/api/transactions/years"), Resolution::MethodNotAllowed { allow: "GET" });
        assert_eq!(resolve("GET", "/api/recurring/1"), Resolution::MethodNotAllowed { allow: "PATCH" });
        assert_eq!(resolve("HEAD", "/api/tags"), Resolution::MethodNotAllowed { allow: "GET" });
        assert_eq!(resolve("GET", "/api/tag%73"), rewrite("/api/tags"));
        let no_match = |rewrite: Option<&str>| Resolution::NoMatch { rewrite: rewrite.map(String::from) };
        assert_eq!(resolve("DELETE", "/api/tags/a%2Fb"), no_match(Some("/api/tags/a/b")));
        assert_eq!(resolve("GET", "/api/tag%73/"), no_match(Some("/api/tags/")));
        assert_eq!(resolve("DELETE", "/api/tags/%2541"), rewrite("/api/tags/%2541"));
        assert_eq!(resolve("GET", "/api/tags/"), no_match(None));
        assert_eq!(resolve("GET", "/api/nope"), no_match(None));
    }
}
