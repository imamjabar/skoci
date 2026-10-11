use axum::response::Html;

pub async fn handle_dashboard() -> Html<&'static str> {
    Html(include_str!("../../../../dashboard/index.html"))
}
