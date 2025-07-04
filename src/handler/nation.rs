use actix_web::{connect, delete, get, head, options, patch, post, put, trace, web, HttpResponse};


// 示例 GET 路由（无参数）
#[get("/")]
pub async fn hello() -> HttpResponse {
    HttpResponse::Ok().body("Hello from auto_route!")
}

// 示例 POST 路由（使用 String 提取器）
#[post("/echo")]
pub async fn echo(body: String) -> HttpResponse {
    HttpResponse::Ok().body(body)
}

#[get("/user/{id}")]
pub async fn get_user(id: web::Path<String>) -> HttpResponse {
    let user_id = id.into_inner();
    HttpResponse::Ok().body(format!("Get user: {}", user_id))
}

#[post("/user")]
pub async fn create_user() -> HttpResponse {
    HttpResponse::Ok().body("User created")
}

#[put("/user/{id}")]
pub async fn update_user(id: web::Path<String>) -> HttpResponse {
    HttpResponse::Ok().body(format!("Updated user: {}", id))
}

#[delete("/user/{id}")]
pub async fn delete_user(id: web::Path<String>) -> HttpResponse {
    HttpResponse::Ok().body(format!("Deleted user: {}", id))
}

#[head("/head")]
pub async fn head_example() -> HttpResponse {
    HttpResponse::Ok().body("HEAD request received")
}

#[connect("/connect")]
pub async fn connect_example() -> HttpResponse {
    HttpResponse::Ok().body("CONNECT request received")
}

#[options("/options")]
pub async fn options_example() -> HttpResponse {
    HttpResponse::Ok().body("OPTIONS request received")
}

#[trace("/trace")]
pub async fn trace_example() -> HttpResponse {
    HttpResponse::Ok().body("TRACE request received")
}

#[patch("/patch")]
pub async fn patch_example() -> HttpResponse {
    HttpResponse::Ok().body("PATCH request received")
}
