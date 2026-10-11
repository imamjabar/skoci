use axum::extract::State;
use axum::response::Response;
use axum::Json;
use skoci_api::error::SkociError;
use skoci_api::requests::DefaultAppRequest;
use skoci_api::responses::DefaultAppResponse;

use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_get_default_app(
    State(state): State<AppState>,
    AuthSubject(_subject): AuthSubject,
) -> Response {
    let app = state.db.get_setting("default_app").ok().flatten();
    api_reply(Ok(DefaultAppResponse { app }))
}

pub async fn handle_set_default_app(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<DefaultAppRequest>,
) -> Response {
    let current_default = state.db.get_setting("default_app").ok().flatten();

    if !subject.is_admin() {
        // Agent can only set default app for its own app, and ONLY when no default is set
        if let Err(err) = subject.require_app(&body.app) {
            return api_reply::<()>(Err(err));
        }

        if current_default.is_some() {
            let err = SkociError::forbidden(
                "Default app is already configured. Only an admin can change it",
                "Use admin credentials / master token to change the default app",
            );
            return api_reply::<()>(Err(err));
        }
    }

    match state.db.list_routes_for_app(&body.app) {
        Ok(routes) if !routes.is_empty() => {}
        Ok(_) => return api_reply::<()>(Err(SkociError::app_not_found(&body.app))),
        Err(e) => return api_reply::<()>(Err(e)),
    }

    if let Err(e) = state.db.set_setting("default_app", &body.app) {
        return api_reply::<()>(Err(e));
    }

    state.router.set_default_app(Some(body.app.clone())).await;

    let _ = state.db.record_audit(
        &subject.identifier(),
        subject.actor_type(),
        "set_default_app",
        &body.app,
        "Server default app updated",
    );

    api_reply(Ok(DefaultAppResponse {
        app: Some(body.app),
    }))
}
