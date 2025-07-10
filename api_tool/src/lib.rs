use actix_web::{put, web, HttpResponse};

pub mod api_tool;

#[put("/lib/{id}")]
pub async fn lib(id: web::Path<String>) -> HttpResponse {
    HttpResponse::Ok().body(format!("Updated user: {}", id.to_string()))
}
