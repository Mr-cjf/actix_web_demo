use actix_web::{get, web, HttpResponse};

#[get("/oem/{id}")]
pub async fn get_ome(id: web::Path<String>) -> HttpResponse {
    let user_id = id.into_inner();
    HttpResponse::Ok().body(format!("Get user: {}", user_id))
}
