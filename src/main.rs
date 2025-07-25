// 必须这样引入宏
#[macro_use]
extern crate route_codegen;
pub mod api;
pub mod handler;
use actix_web::{get, App, HttpResponse, HttpServer};
use log::info;

#[get("/health")]
async fn health() -> HttpResponse {
    info!("health check");
    HttpResponse::Ok().body("health check")
}

// 使用宏生成 configure 函数
generate_configure!("**/src/**/*.rs");
// generate_configure!();
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 初始化日志系统
    env_logger::init();
    info!("🚀 正在启动 Web 服务，监听地址： 0.0.0.0:8080");
    // 设置 RUST_LOG（可选）
    unsafe {
        std::env::set_var("RUST_LOG", "web_demo=info");
    }

    // 启动服务，并注册共享状态
    HttpServer::new(move || App::new().service(health).configure(configure))
        .bind("0.0.0.0:8080")?
        .run()
        .await
}
