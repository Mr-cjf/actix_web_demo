use actix_web::{put, web, HttpResponse};

pub mod agency;
pub mod nation;
pub mod submodules;
// 确保导出所有处理函数

#[put("/mod/{id}")]
pub async fn update_mod(id: web::Path<String>) -> HttpResponse {
    HttpResponse::Ok().body(format!("Updated user: {}", id.to_string()))
}
