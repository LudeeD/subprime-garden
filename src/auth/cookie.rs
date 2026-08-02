use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, Key, PrivateCookieJar, SameSite};

use crate::error::AppError;
use crate::web::AppState;

use super::{SessionData, LOGIN_CSRF_COOKIE, SESSION_COOKIE};

/// Proof that the request carried a valid, unexpired session cookie. Extract
/// this in any admin handler to require auth — extraction itself redirects
/// anonymous requests to the login page.
pub struct AdminSession {
    pub csrf: String,
}

#[axum::async_trait]
impl FromRequestParts<AppState> for AdminSession {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let jar = PrivateCookieJar::<Key>::from_request_parts(parts, state)
            .await
            .map_err(|_| Redirect::to("/admin/login").into_response())?;

        let session: SessionData = jar
            .get(SESSION_COOKIE)
            .and_then(|c| serde_json::from_str(c.value()).ok())
            .ok_or_else(|| Redirect::to("/admin/login").into_response())?;

        if session.is_expired(state.config.auth.session_ttl_days) {
            return Err(Redirect::to("/admin/login").into_response());
        }

        Ok(AdminSession { csrf: session.csrf })
    }
}

impl AdminSession {
    /// Every admin mutation must call this with its form's `csrf_token` field.
    pub fn verify_csrf(&self, submitted: &str) -> Result<(), AppError> {
        if constant_time_eq(self.csrf.as_bytes(), submitted.as_bytes()) {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn session_cookie(session: &SessionData, ttl_days: u32) -> Cookie<'static> {
    let value = serde_json::to_string(session).expect("SessionData always serializes");
    Cookie::build((SESSION_COOKIE, value))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::days(i64::from(ttl_days)))
        .build()
}

pub fn expired_session_cookie() -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .max_age(time::Duration::seconds(0))
        .build()
}

/// Short-lived cookie backing the CSRF token on the login form itself —
/// there's no session yet at that point to carry one.
pub fn login_csrf_cookie(token: &str) -> Cookie<'static> {
    Cookie::build((LOGIN_CSRF_COOKIE, token.to_string()))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .path("/admin/login")
        .max_age(time::Duration::minutes(10))
        .build()
}
