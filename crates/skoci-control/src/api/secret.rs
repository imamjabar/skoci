use axum::extract::{Path, State};
use axum::response::Response;
use axum::Json;
use skoci_api::error::SkociError;
use skoci_api::requests::SecretSetRequest;
use skoci_api::responses::SecretListResponse;
use skoci_core::config::is_valid_env_key;

use crate::http::{api_reply, AuthSubject};
use crate::state::AppState;

pub async fn handle_set_secret(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Json(body): Json<SecretSetRequest>,
) -> Response {
    if let Err(err) = subject.require_app(&body.app) {
        return api_reply::<()>(Err(err));
    }

    if !is_valid_env_key(&body.key) {
        let err = SkociError::bad_request(
            format!("Nama secret key '{}' tidak valid", body.key),
            "Gunakan huruf kapital, angka, dan underscore, contoh: API_KEY atau DB_PASSWORD",
        );
        return api_reply::<()>(Err(err));
    }

    if body.key.starts_with("SKOCI_") {
        let err = SkociError::bad_request(
            "Prefix 'SKOCI_' dicadangkan untuk internal sistem",
            "Gunakan nama key lain tanpa prefix 'SKOCI_'",
        );
        return api_reply::<()>(Err(err));
    }

    if body.value.contains('\n') {
        let err = SkociError::bad_request(
            "Nilai secret tidak boleh mengandung baris baru (newline)",
            "Hapus karakter baris baru atau lakukan base64 encode jika membutuhkan teks multi-baris",
        );
        return api_reply::<()>(Err(err));
    }

    let (ct, nonce) = match state.secret_cipher.encrypt(&body.value) {
        Ok(enc) => enc,
        Err(e) => return api_reply::<()>(Err(e)),
    };

    if let Err(e) = state.db.set_secret(&body.app, &body.key, &ct, &nonce) {
        return api_reply::<()>(Err(e));
    }

    let _ = state.db.record_audit(
        &subject.identifier(),
        subject.actor_type(),
        "secret_set",
        &format!("{}/{}", body.app, body.key),
        "Secret diperbarui",
    );

    api_reply(Ok(()))
}

pub async fn handle_list_secrets(
    State(state): State<AppState>,
    AuthSubject(subject): AuthSubject,
    Path(app): Path<String>,
) -> Response {
    if let Err(err) = subject.require_app(&app) {
        return api_reply::<()>(Err(err));
    }

    match state.db.list_secret_keys_for_app(&app) {
        Ok(keys) => api_reply(Ok(SecretListResponse { app, keys })),
        Err(e) => api_reply::<()>(Err(e)),
    }
}
