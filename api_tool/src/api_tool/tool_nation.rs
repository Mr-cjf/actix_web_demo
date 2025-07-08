use actix_web::{get, HttpResponse};

// 示例 GET 路由（无参数）
#[get("/hello")]
pub async fn hello() -> HttpResponse {
    HttpResponse::Ok().body("Hello from auto_route!")
}
