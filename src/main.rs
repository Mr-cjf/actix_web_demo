// 必须这样引入宏
#[macro_use]
extern crate route_codegen;
pub mod api;
pub mod handler;
use actix_web::{App, HttpServer};

// 使用宏生成 configure 函数
generate_configure!();

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // 初始化日志系统
    env_logger::init();

    // 设置 RUST_LOG（可选）
    unsafe {
        std::env::set_var("RUST_LOG", "web_demo=info");
    }

    println!("Starting HTTP server at http://127.0.0.1:8080");

    // 启动服务，并注册共享状态
    HttpServer::new(move || App::new().configure(configure))
        .bind("127.0.0.1:8080")?
        .run()
        .await
}
