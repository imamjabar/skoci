use axum::extract::State;
use axum::response::Response;

use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_list_audits(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
) -> Response {
    if let Err(err) = subject.require_admin() {
        return api_reply::<()>(Err(err));
    }

    match state.db.list_audits(100) {
        Ok(audits) => api_reply(Ok(audits)),
        Err(e) => api_reply::<()>(Err(e)),
    }
}
