//! Proteção do servidor local contra sites abertos no navegador (CSRF, DNS rebinding):
//! Host e Origin precisam ser o próprio app, e toda rota exige o token da sessão, que
//! chega uma vez na URL (`?t=`) e vira um cookie `HttpOnly; SameSite=Strict`.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use subtle::ConstantTimeEq;

use super::AppState;

pub const COOKIE: &str = "scriba_token";

/// Rotas que dispensam o token (ainda exigem Host válido). O `/health` só devolve o
/// id público da instância, usado pela segunda execução.
const PUBLIC_PATHS: &[&str] = &["/health"];

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Token válido na query: grava o cookie e redireciona para a URL limpa.
    Login {
        location: String,
    },
    Unauthorized,
    Forbidden,
}

pub struct Incoming<'a> {
    pub method: &'a Method,
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub headers: &'a HeaderMap,
}

pub fn decide(req: &Incoming, token: &str, port: u16) -> Decision {
    let own = |value: &str, scheme: &str| {
        [
            format!("{scheme}127.0.0.1:{port}"),
            format!("{scheme}localhost:{port}"),
        ]
        .iter()
        .any(|o| o.eq_ignore_ascii_case(value))
    };
    let header = |name| req.headers.get(name).and_then(|v| v.to_str().ok());

    if !header(header::HOST).is_some_and(|h| own(h, "")) {
        return Decision::Forbidden;
    }
    match header(header::ORIGIN) {
        Some(origin) if !own(origin, "http://") => return Decision::Forbidden,
        // Navegadores mandam Origin em todo POST/PUT/DELETE; sem ele, não é o app.
        None if !matches!(*req.method, Method::GET | Method::HEAD) => {
            return Decision::Forbidden;
        }
        _ => {}
    }
    if PUBLIC_PATHS.contains(&req.path) {
        return Decision::Allow;
    }
    if let Some(given) = query_token(req.query) {
        return if same(given, token) {
            Decision::Login {
                location: req.path.to_string(),
            }
        } else {
            Decision::Unauthorized
        };
    }
    if cookie_token(req.headers).is_some_and(|c| same(c, token)) {
        Decision::Allow
    } else {
        Decision::Unauthorized
    }
}

fn same(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

fn query_token(query: Option<&str>) -> Option<&str> {
    query?.split('&').find_map(|kv| kv.strip_prefix("t="))
}

fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix(COOKIE)?.strip_prefix('='))
}

pub async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let decision = decide(
        &Incoming {
            method: req.method(),
            path: req.uri().path(),
            query: req.uri().query(),
            headers: req.headers(),
        },
        &state.token,
        state.port,
    );
    let mut response = match decision {
        Decision::Allow => next.run(req).await,
        Decision::Login { location } => (
            StatusCode::SEE_OTHER,
            [
                (header::LOCATION, location),
                (
                    header::SET_COOKIE,
                    format!(
                        "{COOKIE}={}; HttpOnly; SameSite=Strict; Path=/",
                        state.token
                    ),
                ),
            ],
        )
            .into_response(),
        Decision::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            "Sessão inválida. Abra o Scriba pelo executável para entrar.",
        )
            .into_response(),
        Decision::Forbidden => (StatusCode::FORBIDDEN, "Acesso negado.").into_response(),
    };
    let headers = response.headers_mut();
    for (name, value) in [
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; img-src 'self' data:; frame-ancestors 'none'; form-action 'self'; base-uri 'none'",
        ),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "abc123";
    const PORT: u16 = 4000;

    fn run(method: Method, path: &str, query: Option<&str>, headers: &[(&str, &str)]) -> Decision {
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            map.append(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        decide(
            &Incoming {
                method: &method,
                path,
                query,
                headers: &map,
            },
            TOKEN,
            PORT,
        )
    }

    const HOST: (&str, &str) = ("host", "127.0.0.1:4000");
    const COOKIE_OK: (&str, &str) = ("cookie", "x=1; scriba_token=abc123");

    #[test]
    fn host_estranho_ou_ausente_e_proibido() {
        assert_eq!(
            run(Method::GET, "/", None, &[COOKIE_OK]),
            Decision::Forbidden
        );
        for host in [
            "evil.com",
            "127.0.0.1:4001",
            "127.0.0.1",
            "localhost.evil.com:4000",
        ] {
            let d = run(Method::GET, "/", None, &[("host", host), COOKIE_OK]);
            assert_eq!(d, Decision::Forbidden, "{host}");
        }
        let d = run(
            Method::GET,
            "/",
            None,
            &[("host", "LOCALHOST:4000"), COOKIE_OK],
        );
        assert_eq!(d, Decision::Allow);
    }

    #[test]
    fn origin_de_outro_site_e_proibido() {
        let d = run(
            Method::GET,
            "/",
            None,
            &[HOST, COOKIE_OK, ("origin", "https://evil.com")],
        );
        assert_eq!(d, Decision::Forbidden);
        let d = run(Method::POST, "/api/theme", None, &[HOST, COOKIE_OK]);
        assert_eq!(d, Decision::Forbidden, "POST sem Origin");
        let ok = ("origin", "http://127.0.0.1:4000");
        assert_eq!(
            run(Method::POST, "/api/theme", None, &[HOST, COOKIE_OK, ok]),
            Decision::Allow
        );
    }

    #[test]
    fn sem_token_e_nao_autorizado() {
        assert_eq!(run(Method::GET, "/", None, &[HOST]), Decision::Unauthorized);
        let wrong = ("cookie", "scriba_token=abc124");
        assert_eq!(
            run(Method::GET, "/", None, &[HOST, wrong]),
            Decision::Unauthorized
        );
        let prefix = ("cookie", "scriba_token=abc");
        assert_eq!(
            run(Method::GET, "/", None, &[HOST, prefix]),
            Decision::Unauthorized
        );
        assert_eq!(
            run(Method::GET, "/", Some("t=x"), &[HOST]),
            Decision::Unauthorized
        );
    }

    #[test]
    fn token_na_query_vira_login() {
        assert_eq!(
            run(Method::GET, "/", Some("a=1&t=abc123"), &[HOST]),
            Decision::Login {
                location: "/".into()
            }
        );
    }

    #[test]
    fn health_dispensa_token_mas_nao_host() {
        assert_eq!(run(Method::GET, "/health", None, &[HOST]), Decision::Allow);
        let evil = ("host", "evil.com");
        assert_eq!(
            run(Method::GET, "/health", None, &[evil]),
            Decision::Forbidden
        );
    }
}
